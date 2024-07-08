//! Everything related to import data into shopware

use crate::api::filter::Criteria;
use crate::api::{Entity, SwApiError, SwErrorBody, SyncAction};
use crate::data::transform::deserialize_row;
use crate::SyncContext;
use csv::StringRecord;
use itertools::Itertools;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// will do blocking file IO, so should be used with `task::spawn_blocking`
pub fn import(context: Arc<SyncContext>) -> anyhow::Result<()> {
    let mut csv_reader = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_path(&context.file)?;
    let headers = csv_reader.headers()?.clone();

    // create an iterator, that processes (CSV) rows (StringRecord) into (usize, StringRecord)
    // where the former is the row index
    let iter = csv_reader
        .into_records()
        .map(|result| match result {
            Ok(record) => record,
            Err(e) => {
                panic!("failed to read CSV record: {}", e);
            }
        })
        .enumerate()
        .take(context.limit.unwrap_or(u64::MAX) as usize);

    // iterate in chunks of Criteria::MAX_LIMIT or less
    let mut join_handles: Vec<JoinHandle<anyhow::Result<()>>> = vec![];
    for sync_values in &iter.chunks(Criteria::MAX_LIMIT as usize) {
        let (row_indices, records_chunk): (Vec<usize>, Vec<StringRecord>) = sync_values.unzip();

        // ToDo: we might want to wait here instead of processing the whole CSV file
        // and then only waiting on the processing / sync requests to finish

        // submit task
        let context = Arc::clone(&context);
        let headers = headers.clone();
        join_handles.push(tokio::spawn(async move {
            let entity_chunk = process_chunk(headers, records_chunk, &context).await?;

            sync_chunk(&row_indices, entity_chunk, &context).await
        }));
    }

    // wait for all the tasks to finish
    tokio::runtime::Handle::current().block_on(async {
        for join_handle in join_handles {
            join_handle.await??;
        }
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

/// deserialize chunk on worker thread
/// and wait for it to finish
async fn process_chunk(
    headers: StringRecord,
    records_chunk: Vec<StringRecord>,
    context: &Arc<SyncContext>,
) -> anyhow::Result<Vec<Entity>> {
    println!("deserialize chunk");
    let context = Arc::clone(context);
    let (worker_tx, worker_rx) = tokio::sync::oneshot::channel::<anyhow::Result<Vec<Entity>>>();
    rayon::spawn(move || {
        let mut entities: Vec<Entity> = Vec::with_capacity(Criteria::MAX_LIMIT as usize);
        for record in records_chunk {
            let entity = match deserialize_row(&headers, record, &context) {
                Ok(e) => e,
                Err(e) => {
                    worker_tx.send(Err(e)).unwrap();
                    return;
                }
            };

            entities.push(entity);
        }

        worker_tx.send(Ok(entities)).unwrap();
    });
    worker_rx.await?
}

async fn sync_chunk(
    row_indices: &[usize],
    mut chunk: Vec<Entity>,
    context: &Arc<SyncContext>,
) -> anyhow::Result<()> {
    match context
        .sw_client
        .sync(&context.schema.entity, SyncAction::Upsert, &chunk)
        .await
    {
        Ok(()) => Ok(()),
        Err(SwApiError::Server(_, error_body)) => {
            remove_invalid_entries_from_chunk(row_indices, &mut chunk, error_body);

            // retry
            context
                .sw_client
                .sync(&context.schema.entity, SyncAction::Upsert, &chunk)
                .await?;
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
    for err in error_body.errors.into_iter() {
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
