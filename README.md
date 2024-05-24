# POC sw-sync-cli
created during the [import / export hackathon](https://shopware.atlassian.net/wiki/spaces/PRODUCT/pages/19887489171/Import+Export+hackathon).

This cli tool written in rust calls the shopware sync api to import an CSV file as fast as possible.

## Usage instructions
1. Have the rust toolchain installed
2. Copy and fill in the `example.credentials.toml` to `.credentials.toml` with integration credentials (admin -> settings -> system -> integrations)
3. `cargo run --release`
