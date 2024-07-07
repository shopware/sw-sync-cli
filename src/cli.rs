//! Definitions for the CLI commands, arguments and help texts
//!
//! Makes heavy use of https://docs.rs/clap/latest/clap/

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Authenticate with a given shopware shop via integration admin API.
    /// Credentials are stored in .credentials.toml in the current working directory.
    Auth {
        /// base URL of the shop
        #[arg(short, long)]
        domain: String,

        /// access_key_id
        #[arg(short, long)]
        id: String,

        /// access_key_secret
        #[arg(short, long)]
        secret: String,
    },

    /// Import data into shopware or export data to a file
    Sync {
        /// Mode (import or export)
        #[arg(value_enum, short, long)]
        mode: SyncMode,

        /// Path to profile schema.yaml
        #[arg(short, long)]
        schema: PathBuf,

        /// Path to data file
        #[arg(short, long)]
        file: PathBuf,

        /// Maximum amount of entities, can be used for debugging
        #[arg(short, long)]
        limit: Option<u64>,

        // Verbose output, used for debugging
        // #[arg(short, long, action = ArgAction::SetTrue)]
        // verbose: bool,
        /// How many requests can be "in-flight" at the same time
        #[arg(short, long, default_value = "10")]
        in_flight_limit: usize,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum SyncMode {
    Import,
    Export,
}
