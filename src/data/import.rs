use crate::api::{SwApiError, SyncAction};
use crate::data::transform::deserialize_row;
use crate::SyncContext;
use itertools::Itertools;
use std::sync::Arc;

/// Might block, so should be used with `task::spawn_blocking`
pub async fn import(context: Arc<SyncContext>) -> anyhow::Result<()> {
    let mut csv_reader = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_path(&context.file)?;
    let headers = csv_reader.headers()?.clone();

    let iter = csv_reader
        .into_records()
        .map(|r| {
            let result = r.expect("failed reading CSV row");

            deserialize_row(&headers, result, &context).expect("deserialize failed")
            // ToDo improve error handling
        })
        .enumerate()
        .take(context.limit.unwrap_or(u64::MAX) as usize);

    let mut join_handles = vec![];
    for sync_values in &iter.chunks(500) {
        let (mut row_indices, mut chunk): (
            Vec<usize>,
            Vec<serde_json::Map<String, serde_json::Value>>,
        ) = sync_values.unzip();

        let context = Arc::clone(&context);
        join_handles.push(tokio::spawn(async move {
            match context.sw_client.sync(&context.schema.entity, SyncAction::Upsert, &chunk).await {
                Ok(()) => Ok(()),
                Err(SwApiError::Server(_, body)) => {
                    for err in body.errors.iter().rev() {
                        const PREFIX: &str = "/write_data/";
                        let (entry_str , remaining_pointer)= &err.source.pointer[PREFIX.len()..].split_once('/').expect("error pointer");
                        let entry: usize = entry_str.parse().expect("error pointer should contain usize");

                        let row_index = row_indices.remove(entry);
                        let row = chunk.remove(entry);
                        println!(
                            "server validation error on row {}: {} Remaining pointer '{}' ignored payload:\n{}",
                            row_index + 2,
                            err.detail,
                            remaining_pointer,
                            serde_json::to_string_pretty(&row)?,
                        );
                    }
                    // retry
                    context.sw_client.sync(&context.schema.entity, SyncAction::Upsert, &chunk).await
                },
                Err(e) => Err(e),
            }
        }));
    }

    for join_handle in join_handles {
        join_handle.await??;
    }

    Ok(())
}
