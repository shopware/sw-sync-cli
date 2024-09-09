//! Everything related to import data into shopware

use crate::api::filter::Criteria;
use crate::api::{Entity, SwApiError, SwError, SwErrorBody, SyncAction};
use crate::data::transform::deserialize_row;
use crate::SyncContext;
use anyhow::{anyhow, Context};
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
                let entity_chunk = match deserialize_chunk(headers, first_index, records_chunk, &context_clone) {
                    Ok(chunk) => chunk,
                    Err(e) => {
                        println!("sync chunk {first_index}..={last_index} (size={chunk_length}) failed to deserialize:\n{e:#}");
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
    first_index: usize,
    records_chunk: Vec<Result<StringRecord, csv::Error>>,
    context: &Arc<SyncContext>,
) -> anyhow::Result<Vec<Entity>> {
    let mut entities: Vec<Entity> = Vec::with_capacity(Criteria::MAX_LIMIT);
    for (record_counter, record) in records_chunk.into_iter().enumerate() {
        let record = record?; // fail on first CSV read failure

        let entity = deserialize_row(
            headers,
            &record,
            &context.profile,
            &context.scripting_environment,
        )
        .with_context(|| format!("error in row {}", record_counter + first_index))?;

        entities.push(entity);
    }

    Ok(entities)
}

fn sync_chunk(
    row_indices: &[usize],
    mut chunk: Vec<Entity>,
    context: &Arc<SyncContext>,
) -> anyhow::Result<()> {
    if let Ok(()) = attempt_chunk_sync_with_retries(row_indices, &mut chunk, context) {
        return Ok(());
    }

    println!("chunk import failed; starting with single row import to filter faulty rows");

    for (entity, index) in chunk.into_iter().zip(row_indices.iter()) {
        match attempt_chunk_sync_with_retries(row_indices, &mut vec![entity], context) {
            Ok(_) => {}
            Err(error) => {
                println!("{error:?}");
                println!("invalid entry at row {index} will be skipped");
            }
        }
    }

    Ok(())
}

fn attempt_chunk_sync_with_retries(
    row_indices: &[usize],
    chunk: &mut Vec<Entity>,
    context: &Arc<SyncContext>,
) -> anyhow::Result<()> {
    let mut try_count = context.try_count.get();
    loop {
        if try_count == 0 {
            return Err(anyhow!("max try count reached"));
        }

        let (error_status, error_body) =
            match context
                .sw_client
                .sync(&context.profile.entity, SyncAction::Upsert, chunk)
            {
                Ok(()) => {
                    return Ok(());
                }
                Err(SwApiError::Server(error_status, error_body)) => (error_status, error_body),
                Err(e) => {
                    return Err(e.into());
                }
            };

        match error_body {
            body if body.check_for_error_code(SwError::ERROR_CODE_DEADLOCK) => {
                println!("deadlock occurred; retry initialized");
                try_count = try_count.saturating_sub(1);
            }
            ref body
                if body
                    .errors
                    .iter()
                    .any(|e| matches!(e, SwError::WriteError { .. })) =>
            {
                println!("write error occurred; retry initialized");
                remove_invalid_entries_from_chunk(row_indices, chunk, body);

                if chunk.is_empty() {
                    return Ok(());
                }

                try_count = try_count.saturating_sub(1);
            }
            body => {
                return Err(SwApiError::Server(error_status, body).into());
            }
        };

        println!("tries remaining: {try_count}")
    }
}

fn remove_invalid_entries_from_chunk(
    row_indices: &[usize],
    chunk: &mut Vec<Entity>,
    error_body: &SwErrorBody,
) {
    let mut to_be_removed = vec![];
    for err in &error_body.errors {
        let (source, detail) = match err {
            SwError::WriteError { source, detail, .. } => (source, detail),
            err => {
                println!("{:?}", err);
                continue;
            }
        };

        const PREFIX: &str = "/write_data/";
        let (entry_str, remaining_pointer) = &source.pointer[PREFIX.len()..]
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
            detail,
            remaining_pointer,
            serde_json::to_string_pretty(&row).unwrap(),
        );
        to_be_removed.push(entry);
    }

    // sort descending to remove by index
    to_be_removed.sort_unstable_by(|a, b| b.cmp(a));

    // filtering duplicate rows
    to_be_removed.dedup();

    for index in to_be_removed {
        chunk.remove(index);
    }
}
