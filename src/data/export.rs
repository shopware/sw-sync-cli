//! Everything related to exporting data out of shopware

use crate::api::filter::Criteria;
use crate::api::SwListResponse;
use crate::data::transform::serialize_entity;
use crate::SyncContext;
use std::cmp;
use std::sync::Arc;

pub fn export(context: Arc<SyncContext>) -> anyhow::Result<()> {
    if !context.associations.is_empty() {
        println!("Using associations: {:#?}", context.associations);
    }

    if !context.profile.filter.is_empty() {
        println!("Using filter: {:#?}", context.profile.filter);
    }

    if !context.profile.sort.is_empty() {
        println!("Using sort: {:#?}", context.profile.sort);
    }

    // retrieve total entity count from shopware and calculate chunk count
    let mut total = context
        .sw_client
        .get_total(&context.profile.entity, &context.profile.filter)?;

    if total == 0 {
        return Err(anyhow::anyhow!("No entities found for export."));
    }

    if let Some(limit) = context.limit {
        total = cmp::min(limit, total);
    }

    let chunk_limit = cmp::min(
        Criteria::MAX_LIMIT,
        usize::try_from(total).expect("64 bit system wide pointers or values smaller than usize"),
    );
    let chunk_count = total.div_ceil(chunk_limit as u64);
    println!(
        "Reading {} of entity '{}' with chunk limit {}, resulting in {} chunks to be processed",
        total, context.profile.entity, chunk_limit, chunk_count
    );

    // spawn writer thread
    let (writer_tx, rx) = std::sync::mpsc::channel();
    let context_clone = Arc::clone(&context);
    let writer = std::thread::spawn(move || write_to_file_worker(rx, &context_clone));

    // Spawn a thread into the thread pool (rayon) for each chunk.
    // fails on first encountered error
    rayon::scope_fifo(|s| {
        for i in 0..chunk_count {
            let context = Arc::clone(&context);
            let writer_tx = std::sync::mpsc::Sender::clone(&writer_tx);
            s.spawn_fifo(move |_| {
                // Unwrap on failure is fine here for now:
                // if something goes wrong during export, this will panic the thread
                // and that panic will bubble up to the main thread
                // We might re-evaluate this with the ticket: ToDo NEXT-37312

                let page = i + 1;
                println!("processing page {page}...");

                let response = send_request(page, chunk_limit, &context).unwrap();
                let result = process_response(page, chunk_limit, response, &context).unwrap();

                // submit data to file writer thread
                writer_tx.send(result).unwrap();
                println!("processed page {page}");
            });
        }
        drop(writer_tx);
    });

    // wait for the writer thread to finish writing to the CSV file
    // Safety:
    // it's fine to unwrap here, because failure would mean a panic inside the thread,
    // thus panicking the main thread is acceptable
    // Note: we still handle the returned result gracefully and bubble up the error in that case
    writer.join().unwrap()?;

    Ok(())
}

fn send_request(
    page: u64,
    chunk_limit: usize,
    context: &SyncContext,
) -> anyhow::Result<SwListResponse> {
    let mut criteria = Criteria {
        page,
        limit: Some(chunk_limit),
        sort: context.profile.sort.clone(),
        filter: context.profile.filter.clone(),
        ..Default::default()
    };

    for association in &context.associations {
        criteria.add_association(association);
    }

    let response = context.sw_client.list(&context.profile.entity, &criteria)?;

    Ok(response)
}

fn process_response(
    page: u64,
    chunk_limit: usize,
    response: SwListResponse,
    context: &SyncContext,
) -> anyhow::Result<(u64, Vec<Vec<String>>)> {
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(chunk_limit);

    for entity in response.data {
        let row = serialize_entity(&entity, &context.profile, &context.scripting_environment)?;
        rows.push(row);
    }

    Ok((page, rows))
}

#[allow(clippy::type_complexity)]
fn write_to_file_worker(
    rx: std::sync::mpsc::Receiver<(u64, Vec<Vec<String>>)>,
    context: &SyncContext,
) -> anyhow::Result<()> {
    let mut csv_writer = csv::WriterBuilder::new()
        .delimiter(b';')
        .from_path(&context.file)?;

    // writer header line
    csv_writer.write_record(get_header_line(context))?;

    // buffer incoming (page, chunk) messages, to process them in order
    let mut buffer = vec![];
    let mut next_page = 1;
    while let Ok(msg) = rx.recv() {
        buffer.push(msg);

        buffer.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        loop {
            match buffer.last() {
                Some(m) if m.0 == next_page => {}
                _ => break,
            }

            // got the next page, so write it
            let (page, rows) = buffer.remove(buffer.len() - 1);
            println!("writing page {page}");

            for row in rows {
                csv_writer.write_record(row)?;
            }
            next_page += 1;
        }
    }

    csv_writer.flush()?;

    Ok(())
}

fn get_header_line(context: &SyncContext) -> Vec<String> {
    let mut columns = vec![];

    for mapping in &context.profile.mappings {
        columns.push(mapping.get_file_column().to_owned());
    }

    columns
}
