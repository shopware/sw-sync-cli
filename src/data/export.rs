use crate::api::Criteria;
use crate::data::transform::serialize_entity;
use crate::SyncContext;
use std::cmp;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Might block, so should be used with `task::spawn_blocking`
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

    let mut total = context
        .sw_client
        .get_total(&context.schema.entity, &context.schema.filter)
        .await?;
    if let Some(limit) = context.limit {
        total = cmp::min(limit, total);
    }

    let chunk_limit = cmp::min(Criteria::MAX_LIMIT, total);
    let mut page = 1;
    let mut counter = 0;
    println!(
        "Reading {} of entity '{}' with chunk limit {}",
        total, context.schema.entity, chunk_limit
    );

    // submit request tasks
    let mut request_tasks = vec![];
    loop {
        if counter >= total {
            break;
        }

        let context = Arc::clone(&context);
        request_tasks.push(tokio::spawn(async move {
            process_request(page, chunk_limit, &context).await
        }));

        page += 1;
        counter += chunk_limit;
    }

    // wait for all request tasks to finish
    write_to_file(request_tasks, &context).await?;

    Ok(())
}

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
        let (page, rows) = handle.await??;
        println!("writing page {}", page);

        for row in rows {
            csv_writer.write_record(row)?;
        }
    }

    csv_writer.flush()?;

    Ok(())
}

async fn process_request(
    page: u64,
    chunk_limit: u64,
    context: &SyncContext,
) -> anyhow::Result<(u64, Vec<Vec<String>>)> {
    println!(
        "fetching page {} of {} with limit {}",
        page, context.schema.entity, chunk_limit
    );
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(chunk_limit as usize);
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
    for entity in response.data {
        let row = serialize_entity(entity, context)?;
        rows.push(row);
    }

    Ok((page, rows))
}

fn get_header_line(context: &SyncContext) -> Vec<String> {
    let mut columns = vec![];

    for mapping in &context.schema.mappings {
        columns.push(mapping.get_file_column().to_owned());
    }

    columns
}
