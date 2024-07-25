# sw-sync-cli

A CLI tool that communicates with the [Shopware admin API](https://shopware.stoplight.io/docs/admin-api) (over an [integration](https://docs.shopware.com/en/shopware-6-en/settings/system/integrationen?category=shopware-6-en/settings/system)) to export data into (CSV) files or import data from (CSV) files.

---

> [!WARNING]  
> This tool is experimental and for now just a proof of concept.

## Overview

- [Features](https://github.com/shopware/sw-sync-cli#features)
- [Installation](https://github.com/shopware/sw-sync-cli#installation)
- [Usage](https://github.com/shopware/sw-sync-cli#usage)
- [License](https://github.com/shopware/sw-sync-cli#license)

## Features

- It's fast, with a focus on performance
- Every entity and field available in the API can be exported / imported
- Supports data filtering and sorting using the API criteria
- Import / Export profiles (schema's) are just `.yaml` files
  - Which can be copied + adapted + shared freely
- Profiles include a scripting engine for arbitrary data transformations
- For now only supports CSV files

## Installation

#### With Cargo ([Rust toolchain](https://www.rust-lang.org/learn/get-started))

```bash
cargo install sw-sync-cli
```

Same command can be used for updates. See [crate](https://crates.io/crates/sw-sync-cli)

#### Manual

head to [GitHub releases](https://github.com/shopware/sw-sync-cli/releases) and download the right binary for your operating system.
Then either execute the binary directly or put it in your `PATH`.

#### Build it from this repository

1. Clone this repository
2. Have the latest [Rust toolchain](https://www.rust-lang.org/learn/get-started) installed
3. Run `cargo build --release` inside the repository root folder
4. You will get your executable here `./target/release/sw-sync-cli`

## Usage

1. Set up an [integration](https://docs.shopware.com/en/shopware-6-en/settings/system/integrationen?category=shopware-6-en/settings/system) inside shopware.
2. Call `sw-sync-cli auth` with the required arguments (credentials)

> [!WARNING]  
> This will create a `.credentials.toml` file in your current working directory.
> This file contains your credentials in plain text, you might want to remove it again after you are done syncing.

3. Call `sw-sync-cli sync` in either `-m import` or `-m export` mode, with a profile (`schema.yaml`) and data file `data.csv`

> [!Note]  
> You can call `sw-sync-cli help` at any time to get more information

#### Profiles (sync schema)

To get started take a look at [Profiles in this repository](https://github.com/shopware/sw-sync-cli/tree/main/profiles).
The structure of a profile (sync schema) `.yaml` is as follows:

```yaml
entity: product

# optional filtering, only applied on export
filter:
  # export main products (parentId = NULL) only
  - type: "equals"
    field: "parentId"
    value: null

# optional sorting, only applied on export
sort:
  - field: "name"
    order: "ASC"

# optional additional associations (that you need in your deserialization script)
# note: entity_path associations are already added by default
# only applied on export
associations:
  - "cover"

# mappings can either be
# - by entity_path
# - by key
# the latter needs to be resolved by custom scripts
mappings:
  - file_column: "id"
    entity_path: "id"
  - file_column: "name (default language)"
    entity_path: "name"
  - file_column: "product number"
    entity_path: "productNumber"
  - file_column: "stock"
    entity_path: "stock"
  - file_column: "tax id"
    entity_path: "taxId"
  - file_column: "tax rate"
    # entity path can resolve "To-One-Associations" of any depth
    entity_path: "tax.taxRate"
  - file_column: "manufacturer name"
    # They can also use the optional chaining '?.' operator to fall back to null
    # if the association is null
    entity_path: "manufacturer?.name"
  - file_column: "manufacturer id"
    # for importing, you also need the association id in the association object
    entity_path: "manufacturer?.id"
  - file_column: "gross price EUR"
    key: "gross_price_eur"
  - file_column: "net price EUR"
    key: "net_price_eur"

# optional serialization script, which is called once per entity
serialize_script: |
  // See https://rhai.rs/book/ for scripting language documentation
  // you receive an entity object, which consists of the whole entity API response for that single entity
  // you also receive an empty row object where the specified keys above are missing (you need to set them)
  // the other simple mappings are executed (added to the row object) after this script

  // debugging utils
  // debug(entity); // contains the full entity object from the API (can be huge!)
  // print(row); // will be empty

  // Use 'get_default' to look up a value equivalent to Defaults.php
  let default_currency = get_default("CURRENCY");
  let price = entity.price.find(|p| p.currencyId == default_currency);
  row.gross_price_eur = price.gross;
  row.net_price_eur = price.net;

# optional deserialization script, which is called once per entity
deserialize_script: |
  // See https://rhai.rs/book/ for scripting language documentation
  // you receive 'row' as an object that has all the keys defined above with the corresponding value
  // you also receive an empty entity object, where you need to resolve your keys
  // the other simple mappings are executed (added to the entity object) after this script

  // print(entity); // will be empty
  // debug(row); // will contain only the specified keys + their values

  entity.price = [];
  entity.price.push(#{
    gross: row.gross_price_eur,
    net: row.net_price_eur,
    linked: true,
    currencyId: get_default("CURRENCY"),
  });
```

## License

`sw-sync-cli` is distributed under the terms of the MIT License.  
See the [LICENSE](./LICENSE) file for details.
