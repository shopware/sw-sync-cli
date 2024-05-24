use crate::api::{SwClient, SyncAction};
use crate::config::Credentials;
use anyhow::Context;
use itertools::Itertools;
use serde_json::json;
use std::time::Instant;

mod api;
mod config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let start_instant = Instant::now();
    let payload_size = std::env::args().nth(1).map_or(200usize, |s| {
        s.parse()
            .expect("invalid argument, provide a number for payload_size")
    });
    let credentials = tokio::fs::read_to_string("./.credentials.toml")
        .await
        .context("can't read ./.credentials.toml")?;
    let credentials: Credentials = toml::from_str(&credentials)?;
    let currency_id = "b7d2554b0ce847cd82f3ac9bd1c0dfca";

    let sw_client = SwClient::new(credentials).await?;

    // todo move blocking call to separate thread
    let mut csv_reader = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_path("./data/10kProducts.csv")?;
    let headers = csv_reader.headers()?.clone();
    println!("CSV headers: {:?}", headers);

    let iter = csv_reader.records().map(|r| {
        let result = r.unwrap();

        let sync_product = json!({
            "id": result[0],
            "taxId": result[5],
            "price": [
                {
                    "currencyId": currency_id,
                    "net": result[1].parse::<f64>().unwrap(),
                    "gross": result[2].parse::<f64>().unwrap(),
                    "linked": false,
                }
            ],
            "name": result[6],
            "productNumber": result[3],
            "stock": result[4].parse::<i32>().unwrap(),
        });

        sync_product
    });

    let mut join_handles = vec![];
    for sync_values in &iter.chunks(payload_size) {
        let chunk: Vec<serde_json::Value> = sync_values.collect();
        let sw_client = sw_client.clone();
        join_handles.push(tokio::spawn(async move {
            sw_client.sync("product", SyncAction::Upsert, chunk).await
        }));
    }

    for join_handle in join_handles {
        join_handle.await??;
    }

    println!("Finished after {} ms", start_instant.elapsed().as_millis());
    Ok(())
}
