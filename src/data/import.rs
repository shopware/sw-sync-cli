//! Everything related to import data into shopware

use crate::api::{Entity, SwApiError, SyncAction};
use crate::data::transform::deserialize_row;
use crate::SyncContext;
use anyhow::anyhow;
use itertools::Itertools;
use std::sync::Arc;

/// Might block, so should be used with `task::spawn_blocking`
pub async fn import(context: Arc<SyncContext>) -> anyhow::Result<()> {
    let mut csv_reader = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_path(&context.file)?;
    let headers = csv_reader.headers()?.clone();

    // create an iterator, that processes (CSV) rows (StringRecord) into (usize, anyhow::Result<Entity>)
    // where the former is the row index
    let iter = csv_reader
        .into_records()
        .map(|r| match r {
            Ok(row) => deserialize_row(&headers, row, &context),
            Err(e) => Err(anyhow!(e)),
        })
        .enumerate()
        .take(context.limit.unwrap_or(u64::MAX) as usize);

    // iterate in chunks of 500 or less
    let mut join_handles = vec![];
    for sync_values in &iter.chunks(500) {
        let (mut row_indices, chunk): (Vec<usize>, Vec<anyhow::Result<Entity>>) =
            sync_values.unzip();

        // for now fail on first invalid row
        // currently the most likely deserialization failure is not finding the column / CSV header
        // ToDo: we might want to handle the errors more gracefully here and don't stop on first error
        let mut valid_chunk = chunk.into_iter().collect::<anyhow::Result<Vec<Entity>>>()?;

        // submit sync task
        let context = Arc::clone(&context);
        join_handles.push(tokio::spawn(async move {
            match context.sw_client.sync(&context.schema.entity, SyncAction::Upsert, &valid_chunk).await {
                Ok(()) => Ok(()),
                Err(SwApiError::Server(_, body)) => {
                    for err in body.errors.iter().rev() {
                        const PREFIX: &str = "/write_data/";
                        let (entry_str , remaining_pointer)= &err.source.pointer[PREFIX.len()..].split_once('/').expect("error pointer");
                        let entry: usize = entry_str.parse().expect("error pointer should contain usize");

                        let row_index = row_indices.remove(entry);
                        let row = valid_chunk.remove(entry);
                        println!(
                            "server validation error on row {}: {} Remaining pointer '{}' ignored payload:\n{}",
                            row_index + 2,
                            err.detail,
                            remaining_pointer,
                            serde_json::to_string_pretty(&row)?,
                        );
                    }
                    // retry
                    context.sw_client.sync(&context.schema.entity, SyncAction::Upsert, &valid_chunk).await
                },
                Err(e) => Err(e),
            }
        }));
    }

    // wait for all the sync tasks to finish
    for join_handle in join_handles {
        join_handle.await??;
    }

    Ok(())
}
