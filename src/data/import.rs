//! Everything related to import data into shopware

use crate::api::filter::Criteria;
use crate::api::{Entity, SwApiError, SwErrorBody, SyncAction};
use crate::data::transform::deserialize_row;
use crate::SyncContext;
use csv::StringRecord;
use itertools::Itertools;
use std::sync::Arc;

pub fn import(context: Arc<SyncContext>) -> anyhow::Result<()> {
    let mut csv_reader = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_path(&context.file)?;
    let headers = csv_reader.headers()?.clone();
    let chunked_iter = csv_reader
        .into_records()
        .enumerate()
        // limit how much CSV rows get loaded into memory at once (one file chunk)
        .chunks(Criteria::MAX_LIMIT * context.in_flight_limit * 2);

    // process one big file chunk of a potentially big CSV file at a time
    for file_chunk in &chunked_iter {
        let file_chunk: Vec<(usize, Result<StringRecord, csv::Error>)> = file_chunk.collect();
        let first_index = file_chunk.first().map_or(0, |t| t.0);
        let last_index = file_chunk.last().map_or(0, |t| t.0);
        let chunk_length = file_chunk.len();

        println!("file chunk {first_index}..={last_index} (size={chunk_length}) was read from CSV into memory");
        process_file_chunk(&headers, file_chunk, &context)?;
        println!("file chunk {first_index}..={last_index} (size={chunk_length}) finished and cleared from memory");
    }

    Ok(())
}

fn process_file_chunk(
    headers: &StringRecord,
    file_chunk: Vec<(usize, Result<StringRecord, csv::Error>)>,
    context: &Arc<SyncContext>,
) -> anyhow::Result<()> {
    rayon::scope_fifo(|s| {
        // split the big file_chunk into smaller chunks that fit in single sync requests
        // and iterate over them, spawning a processing tasks for each sync chunk
        let chunked_iter = file_chunk.into_iter().chunks(Criteria::MAX_LIMIT);
        for chunk in &chunked_iter {
            let (row_indices, records_chunk): (Vec<usize>, Vec<Result<StringRecord, csv::Error>>) =
                chunk.unzip();
            let first_index = *row_indices.first().unwrap_or(&0);
            let last_index = *row_indices.last().unwrap_or(&0);
            let chunk_length = records_chunk.len();

            let context_clone = Arc::clone(context);
            let headers = &headers;
            s.spawn_fifo(move |_| {
                println!("sync chunk {first_index}..={last_index} (size={chunk_length}) is now being deserialized");
                let entity_chunk = match deserialize_chunk(headers, records_chunk, &context_clone) {
                    Ok(chunk) => chunk,
                    Err(e) => {
                        println!("sync chunk {first_index}..={last_index} (size={chunk_length}) failed to deserialize:\n{e}");
                        return;
                    }
                };

                println!("sync chunk {first_index}..={last_index} (size={chunk_length}) is now being synced to shopware");
                if let Err(e) = sync_chunk(&row_indices, entity_chunk, &context_clone) {
                    println!("sync chunk {first_index}..={last_index} (size={chunk_length}) failed to be synced over API:\n{e}");
                }
            });
        }

        Ok(())
    })
}

fn deserialize_chunk(
    headers: &StringRecord,
    records_chunk: Vec<Result<StringRecord, csv::Error>>,
    context: &Arc<SyncContext>,
) -> anyhow::Result<Vec<Entity>> {
    let mut entities: Vec<Entity> = Vec::with_capacity(Criteria::MAX_LIMIT);
    for record in records_chunk {
        let record = record?; // fail on first CSV read failure

        let entity = match deserialize_row(
            headers,
            &record,
            &context.profile,
            &context.scripting_environment,
        ) {
            Ok(e) => e,
            Err(e) => {
                return Err(e);
            }
        };

        entities.push(entity);
    }

    Ok(entities)
}

fn sync_chunk(
    row_indices: &[usize],
    mut chunk: Vec<Entity>,
    context: &Arc<SyncContext>,
) -> anyhow::Result<()> {
    match context
        .sw_client
        .sync(&context.profile.entity, SyncAction::Upsert, &chunk)
    {
        Ok(()) => Ok(()),
        Err(SwApiError::Server(_, error_body)) => {
            remove_invalid_entries_from_chunk(row_indices, &mut chunk, error_body);

            // retry
            context
                .sw_client
                .sync(&context.profile.entity, SyncAction::Upsert, &chunk)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn remove_invalid_entries_from_chunk(
    row_indices: &[usize],
    chunk: &mut Vec<Entity>,
    error_body: SwErrorBody,
) {
    let mut to_be_removed = vec![];
    for err in error_body.errors {
        const PREFIX: &str = "/write_data/";
        let (entry_str, remaining_pointer) = &err.source.pointer[PREFIX.len()..]
            .split_once('/')
            .expect("error pointer");
        let entry: usize = entry_str
            .parse()
            .expect("error pointer should contain usize");

        let row_index = row_indices
            .get(entry)
            .expect("error pointer should have a entry in row_indices");
        let row_line_number = row_index + 2;
        let row = chunk
            .get(entry)
            .expect("error pointer should have a entry in chunk");
        println!(
            "server validation error on (CSV) line {}: {} Remaining pointer '{}' failed payload:\n{}",
            row_line_number,
            err.detail,
            remaining_pointer,
            serde_json::to_string_pretty(&row).unwrap(),
        );
        to_be_removed.push(entry);
    }

    // sort descending to remove by index
    to_be_removed.sort_unstable_by(|a, b| b.cmp(a));
    for index in to_be_removed {
        chunk.remove(index);
    }
}
