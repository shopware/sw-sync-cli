//! Everything related to exporting data out of shopware

use crate::api::filter::Criteria;
use crate::api::SwListResponse;
use crate::data::transform::serialize_entity;
use crate::SyncContext;
use std::cmp;
use std::sync::Arc;
use tokio::task::JoinHandle;

pub async fn export(context: Arc<SyncContext>) -> anyhow::Result<()> {
    if !context.associations.is_empty() {
        println!("Using associations: {:#?}", context.associations);
    }
    
    if !context.schema.filter.is_empty() {
        println!("Using filter: {:#?}", context.schema.filter);
    }
    
    if !context.schema.sort.is_empty() {
        println!("Using sort: {:#?}", context.schema.sort);
    }

    // retrieve total entity count from shopware and calculate chunk count
    let mut total = context
        .sw_client
        .get_total(&context.schema.entity, &context.schema.filter)
        .await?;
    
    if let Some(limit) = context.limit {
        total = cmp::min(limit, total);
    }
    
    let chunk_limit = cmp::min(Criteria::MAX_LIMIT, total);
    let chunk_count = total.div_ceil(chunk_limit);
    println!(
        "Reading {} of entity '{}' with chunk limit {}, resulting in {} chunks to be processed",
        total, context.schema.entity, chunk_limit, chunk_count
    );

    // submit request tasks
    #[allow(clippy::type_complexity)]
    let mut request_tasks: Vec<JoinHandle<anyhow::Result<(u64, Vec<Vec<String>>)>>> = vec![];

    for i in 0..chunk_count {
        let page = i + 1;

        let context = Arc::clone(&context);
        request_tasks.push(tokio::spawn(async move {
            let response = send_request(page, chunk_limit, &context).await?;

            // move actual response processing / deserialization to worker thread pool
            // and wait for it to finish
            let (worker_tx, worker_rx) =
                tokio::sync::oneshot::channel::<anyhow::Result<(u64, Vec<Vec<String>>)>>();

            rayon::spawn(move || {
                let result = process_response(page, chunk_limit, response, &context);
                worker_tx.send(result).unwrap();
            });

            worker_rx.await?
        }));
    }

    // wait for all tasks to finish, one after the other, in order,
    // and write them to the target file (blocking IO)
    tokio::task::spawn_blocking(|| async move { write_to_file(request_tasks, &context).await })
        .await?
        .await?;

    Ok(())
}

async fn send_request(
    page: u64,
    chunk_limit: u64,
    context: &SyncContext,
) -> anyhow::Result<SwListResponse> {
    let mut criteria = Criteria {
        page,
        limit: chunk_limit,
        sort: context.schema.sort.clone(),
        filter: context.schema.filter.clone(),
        ..Default::default()
    };

    for association in &context.associations {
        criteria.add_association(association);
    }

    let response = context
        .sw_client
        .list(&context.schema.entity, &criteria)
        .await?;

    Ok(response)
}

fn process_response(
    page: u64,
    chunk_limit: u64,
    response: SwListResponse,
    context: &SyncContext,
) -> anyhow::Result<(u64, Vec<Vec<String>>)> {
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(chunk_limit as usize);

    for entity in response.data {
        let row = serialize_entity(entity, context)?;
        rows.push(row);
    }

    Ok((page, rows))
}

#[allow(clippy::type_complexity)]
async fn write_to_file(
    worker_handles: Vec<JoinHandle<anyhow::Result<(u64, Vec<Vec<String>>)>>>,
    context: &SyncContext,
) -> anyhow::Result<()> {
    let mut csv_writer = csv::WriterBuilder::new()
        .delimiter(b';')
        .from_path(&context.file)?;

    // writer header line
    csv_writer.write_record(get_header_line(context))?;

    for handle in worker_handles {
        // ToDo: we might want to handle the errors more gracefully here and don't stop on first error
        let (page, rows) = handle.await??;
        println!("writing page {}", page);

        for row in rows {
            csv_writer.write_record(row)?;
        }
    }

    csv_writer.flush()?;

    Ok(())
}

fn get_header_line(context: &SyncContext) -> Vec<String> {
    let mut columns = vec![];

    for mapping in &context.schema.mappings {
        columns.push(mapping.get_file_column().to_owned());
    }

    columns
}
