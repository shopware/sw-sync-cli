//! Definitions for the CLI commands, arguments and help texts
//!
//! Makes heavy use of <https://docs.rs/clap/latest/clap/>

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::string::ToString;

#[derive(Debug, PartialEq, Eq, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
pub enum Commands {
    /// Trigger indexing of all registered indexer in shopware asynchronously.
    Index {
        // Array of indexer names to be skipped
        #[arg(short, long)]
        skip: Vec<String>,
    },

    /// Copy all default profiles to current folder
    CopyProfiles {
        /// Output path
        #[arg(short, long)]
        path: Option<PathBuf>,

        /// Overwrite existing profiles
        #[arg(short, long)]
        force: bool,

        /// List all available profiles
        #[arg(short, long)]
        list: bool,
    },

    /// Authenticate with a given shopware shop via integration admin API.
    /// Credentials are stored in .credentials.toml in the current working directory.
    Auth {
        /// base URL of the shop
        #[arg(short, long)]
        domain: String,

        /// integration access key id
        #[arg(short, long)]
        id: String,

        /// integration access key secret
        #[arg(short, long)]
        secret: String,
    },

    /// Import data into shopware or export data to a file
    Sync {
        /// Mode (import or export)
        #[arg(value_enum, short, long)]
        mode: SyncMode,

        /// Path to profile.yaml
        #[arg(short, long)]
        profile: PathBuf,

        /// Path to data file
        #[arg(short, long)]
        file: PathBuf,

        /// Maximum amount of entities, can be used for debugging and is optional
        #[arg(short, long)]
        limit: Option<u64>,

        // Disable triggering indexer after sync ended successfully
        #[arg(value_enum, short, long, default_value = "false")]
        disable_index: bool,

        // Verbose output, used for debugging
        // #[arg(short, long, action = ArgAction::SetTrue)]
        // verbose: bool,
        /// How many requests can be "in-flight" at the same time
        #[arg(short, long, default_value = in_flight_limit_default_as_string())]
        in_flight_limit: usize,
    },
}

pub const DEFAULT_IN_FLIGHT: usize = 10;

fn in_flight_limit_default_as_string() -> String {
    DEFAULT_IN_FLIGHT.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SyncMode {
    Import,
    Export,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_cli_arg_parsing() {
        let args = vec![
            "sw-sync-cli",
            "sync",
            "-m",
            "import",
            "--profile",
            "my_profile.yaml",
            "--file",
            "./output.csv",
        ];

        let cli = match Cli::try_parse_from(args) {
            Ok(cli) => cli,
            Err(e) => {
                panic!("Failed to parse cli args: {e}");
            }
        };

        assert_eq!(
            cli,
            Cli {
                command: Commands::Sync {
                    mode: SyncMode::Import,
                    profile: "my_profile.yaml".into(),
                    file: "./output.csv".into(),
                    limit: None,
                    disable_index: false,
                    in_flight_limit: DEFAULT_IN_FLIGHT,
                },
            }
        );
    }
}
