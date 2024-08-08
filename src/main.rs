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
    pub scripting_environment: ScriptingEnvironment,
    pub associations: HashSet<String>,
    pub in_flight_limit: usize,
}

fn main() -> anyhow::Result<()> {
    let start_instant = Instant::now();
    let cli = Cli::parse();

    match cli.command {
        Commands::Index { skip } => {
            index(skip)?;
            println!("Successfully triggered indexing.");
        }
        Commands::CopyProfiles { force, list, path } => {
            copy_profiles(force, list, path);
        }
        Commands::Auth { domain, id, secret } => {
            auth(domain, id, secret)?;
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
            rayon::ThreadPoolBuilder::new()
                .num_threads(in_flight_limit)
                .build_global()
                .unwrap();
            println!("using at most {in_flight_limit} number of threads in a pool");
            let context = create_context(profile, file, limit, in_flight_limit)?;

            match mode {
                SyncMode::Import => {
                    import(Arc::new(context))?;

                    println!("Imported successfully");
                    if disable_index {
                        println!("Indexing was skipped, you might want to run the indexers in your shop later. Go to Settings -> System -> Caches & indexes");
                        println!("Or simply run: sw-sync-cli index");
                    } else {
                        println!("Triggering indexing...");
                        index(vec![])?;
                        println!("Successfully triggered indexing.");
                    }
                }
                SyncMode::Export => {
                    export(Arc::new(context))?;

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

fn index(skip: Vec<String>) -> anyhow::Result<()> {
    let credentials = Credentials::read_credentials()?;

    let sw_client = SwClient::new(credentials)?;
    sw_client.index(skip)?;

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

fn auth(domain: String, id: String, secret: String) -> anyhow::Result<()> {
    let credentials = Credentials {
        base_url: domain.trim_end_matches('/').to_string(),
        access_key_id: id,
        access_key_secret: secret,
    };

    // check if credentials work
    let _ = SwClient::new(credentials.clone())?;

    // write them to file
    let serialized = toml::to_string(&credentials)?;
    std::fs::write("./.credentials.toml", serialized)?;

    Ok(())
}

fn create_context(
    profile_path: PathBuf,
    file: PathBuf,
    limit: Option<u64>,
    in_flight_limit: usize,
) -> anyhow::Result<SyncContext> {
    let profile = Profile::read_profile(profile_path)?;
    let mut associations = profile.associations.clone();
    for mapping in &profile.mappings {
        if let Mapping::ByPath(by_path) = mapping {
            if let Some((association, _field)) = by_path.entity_path.rsplit_once('.') {
                associations.insert(association.trim_end_matches('?').to_owned());
            }
        }
    }

    let credentials = Credentials::read_credentials()?;
    let sw_client = SwClient::new(credentials)?;

    let api_schema = sw_client.entity_schema();
    let entity = &profile.entity;

    validate_paths_for_entity(entity, &profile.mappings, &api_schema?)?;

    // ToDo: create lookup table for currencies?
    let language_list = sw_client.get_languages()?;

    let scripting_environment = prepare_scripting_environment(
        &profile.serialize_script,
        &profile.deserialize_script,
        language_list,
    )?;

    Ok(SyncContext {
        sw_client,
        profile,
        file,
        limit,
        scripting_environment,
        associations,
        in_flight_limit,
    })
}
