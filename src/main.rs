use crate::api::SwClient;
use crate::cli::{Cli, Commands, SyncMode};
use crate::config_file::{Credentials, Mapping, Schema};
use crate::data::validate_paths_for_entity;
use crate::data::{export, import, prepare_scripting_environment, ScriptingEnvironment};
use anyhow::Context;
use clap::Parser;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

mod api;
mod cli;
mod config_file;
mod data;

include!(concat!(env!("OUT_DIR"), "/profiles.rs"));

#[derive(Debug)]
pub struct SyncContext {
    pub sw_client: SwClient,
    pub schema: Schema,
    /// specifies the input or output file
    pub file: PathBuf,
    pub limit: Option<u64>,
    pub in_flight_limit: usize,
    pub scripting_environment: ScriptingEnvironment,
    pub associations: HashSet<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let start_instant = Instant::now();
    let cli = Cli::parse();

    match cli.command {
        Commands::Index { skip } => {
            index(skip).await?;
            println!("Successfully triggered indexing.");
        }
        Commands::CopyProfiles { force, list } => {
            copy_profiles(force, list);

            // TODO: add actual path to folder of profiles
            println!("Successfully copied profiles.")
        }
        Commands::Auth { domain, id, secret } => {
            auth(domain, id, secret).await?;
            println!("Successfully authenticated. You can continue with other commands now.")
        }
        Commands::Sync {
            mode,
            schema,
            file,
            limit,
            disable_index,
            // verbose,
            in_flight_limit,
        } => {
            let context = create_context(schema, file, limit, in_flight_limit).await?;

            match mode {
                SyncMode::Import => {
                    tokio::task::spawn_blocking(move || import(Arc::new(context))).await??;

                    println!("Imported successfully");
                    println!("You might want to run the indexers in your shop now. Go to Settings -> System -> Caches & indexes");
                }
                SyncMode::Export => {
                    export(Arc::new(context)).await?;

                    println!("Exported successfully");
                }
            }

            if !disable_index {
                index(vec![]).await?;
                println!("Successfully triggered indexing.");
            }
        }
    }

    println!(
        "This whole command executed in {:.3}s",
        start_instant.elapsed().as_secs_f32()
    );

    Ok(())
}

async fn index(skip: Vec<String>) -> anyhow::Result<()> {
    let credentials = Credentials::read_credentials().await?;

    let sw_client = SwClient::new(credentials, SwClient::DEFAULT_IN_FLIGHT).await?;
    sw_client.index(skip).await?;

    Ok(())
}

pub fn copy_profiles(force: bool, list: bool) {
    for (name, content) in PROFILES {
        if list {
            println!("Profile: {}", name);
        }

        // TODO: normal mode

        // TODO: force mode

        if force {
            let dest_path = format!("./output/{}", name);
            std::fs::create_dir_all("./output").unwrap(); // Ensure the output directory exists
            std::fs::write(dest_path, content).unwrap();
        }
    }
}

async fn auth(domain: String, id: String, secret: String) -> anyhow::Result<()> {
    let credentials = Credentials {
        base_url: domain.trim_end_matches('/').to_string(),
        access_key_id: id,
        access_key_secret: secret,
    };

    // check if credentials work
    let _ = SwClient::new(credentials.clone(), SwClient::DEFAULT_IN_FLIGHT).await?;

    // write them to file
    let serialized = toml::to_string(&credentials)?;
    tokio::fs::write("./.credentials.toml", serialized).await?;

    Ok(())
}

async fn create_context(
    schema: PathBuf,
    file: PathBuf,
    limit: Option<u64>,
    in_flight_limit: usize,
) -> anyhow::Result<SyncContext> {
    let serialized_schema = tokio::fs::read_to_string(schema)
        .await
        .context("No provided schema file not found")?;
    let schema: Schema = serde_yaml::from_str(&serialized_schema)?;
    let mut associations = schema.associations.clone();
    for mapping in &schema.mappings {
        if let Mapping::ByPath(by_path) = mapping {
            if let Some((association, _field)) = by_path.entity_path.rsplit_once('.') {
                associations.insert(association.trim_end_matches('?').to_owned());
            }
        }
    }

    let credentials = Credentials::read_credentials().await?;
    let sw_client = SwClient::new(credentials, in_flight_limit).await?;

    let api_schema = sw_client.entity_schema().await;
    let entity = &schema.entity;

    validate_paths_for_entity(entity, &schema.mappings, &api_schema?)?;

    // ToDo: create lookup table for languages + currencies?

    let scripting_environment =
        prepare_scripting_environment(&schema.serialize_script, &schema.deserialize_script)?;

    Ok(SyncContext {
        sw_client,
        schema,
        scripting_environment,
        file,
        limit,
        in_flight_limit,
        associations,
    })
}
