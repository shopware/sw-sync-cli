use crate::api::SwClient;
use crate::cli::{Cli, Commands, SyncMode};
use crate::config_file::{Credentials, Mapping, Profile, DEFAULT_PROFILES};
use crate::data::validate_paths_for_entity;
use crate::data::{export, import, prepare_scripting_environment, ScriptingEnvironment};
use clap::Parser;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

mod api;
mod cli;
mod config_file;
mod data;

#[derive(Debug)]
pub struct SyncContext {
    pub sw_client: SwClient,
    pub profile: Profile,
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
        Commands::CopyProfiles { force, list, path } => {
            copy_profiles(force, list, path);
        }
        Commands::Auth { domain, id, secret } => {
            auth(domain, id, secret).await?;
            println!("Successfully authenticated. You can continue with other commands now.");
        }
        Commands::Sync {
            mode,
            profile,
            file,
            limit,
            disable_index,
            // verbose,
            in_flight_limit,
        } => {
            let context = create_context(profile, file, limit, in_flight_limit).await?;

            match mode {
                SyncMode::Import => {
                    tokio::task::spawn_blocking(move || import(Arc::new(context))).await??;

                    println!("Imported successfully");
                    if disable_index {
                        println!("Indexing was skipped, you might want to run the indexers in your shop later. Go to Settings -> System -> Caches & indexes");
                        println!("Or simply run: sw-sync-cli index");
                    } else {
                        println!("Triggering indexing...");
                        index(vec![]).await?;
                        println!("Successfully triggered indexing.");
                    }
                }
                SyncMode::Export => {
                    export(Arc::new(context)).await?;

                    println!("Exported successfully");
                }
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

pub fn copy_profiles(force: bool, list: bool, path: Option<PathBuf>) {
    if list {
        println!("Available profiles:");

        for profile in DEFAULT_PROFILES {
            println!("- {}", profile.0);
        }

        return;
    }

    let dir_path = if let Some(path) = path {
        if path.extension().is_some() {
            eprintln!("Path is not a directory: {path:?}");
            return;
        }

        path
    } else {
        PathBuf::from("./profiles")
    };

    if let Err(e) = fs::create_dir_all(&dir_path) {
        eprintln!("Failed to create directory: {e}");
        return;
    }

    for (name, content) in DEFAULT_PROFILES {
        let dest_path = dir_path.join(name);

        if dest_path.exists() && !force {
            eprintln!("File {name} already exists. Use --force to overwrite.");
            continue;
        }

        match fs::write(&dest_path, content) {
            Ok(()) => println!("Copied profile: {name} -> {dest_path:?}"),
            Err(e) => eprintln!("Failed to write file {name}: {e}"),
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
    profile_path: PathBuf,
    file: PathBuf,
    limit: Option<u64>,
    in_flight_limit: usize,
) -> anyhow::Result<SyncContext> {
    let profile = Profile::read_profile(profile_path).await?;
    let mut associations = profile.associations.clone();
    for mapping in &profile.mappings {
        if let Mapping::ByPath(by_path) = mapping {
            if let Some((association, _field)) = by_path.entity_path.rsplit_once('.') {
                associations.insert(association.trim_end_matches('?').to_owned());
            }
        }
    }

    let credentials = Credentials::read_credentials().await?;
    let sw_client = SwClient::new(credentials, in_flight_limit).await?;

    let api_schema = sw_client.entity_schema().await;
    let entity = &profile.entity;

    validate_paths_for_entity(entity, &profile.mappings, &api_schema?)?;

    // ToDo: create lookup table for languages + currencies?

    let scripting_environment =
        prepare_scripting_environment(&profile.serialize_script, &profile.deserialize_script)?;

    Ok(SyncContext {
        sw_client,
        profile,
        file,
        limit,
        in_flight_limit,
        scripting_environment,
        associations,
    })
}
