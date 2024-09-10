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
- Every entity and field available in the Shopware API can be exported / imported
- Supports data filtering and sorting using the API criteria
- Import / Export profiles are just `.yaml` files
  - That define the way data is processed for import and export
  - Which can be copied + adapted + shared freely
  - They include a scripting engine for arbitrary data transformations
- For now only supports CSV data files

## Installation

### With Cargo ([Rust toolchain](https://www.rust-lang.org/learn/get-started))

```bash
cargo install sw-sync-cli --locked
```

Same command can be used for updates.
This command will build the executable (in release mode) and put it into your `PATH` (where all cargo binaries are).
See [crate](https://crates.io/crates/sw-sync-cli)

### Manual

head to [GitHub releases](https://github.com/shopware/sw-sync-cli/releases) and download the right binary for your operating system.
Then either execute the binary directly or put it in your `PATH`.

### Build it from this repository

1. Clone this repository
2. Have the latest [Rust toolchain](https://www.rust-lang.org/learn/get-started) installed
3. Run `cargo build --release` inside the repository root folder
4. You will get your executable here `./target/release/sw-sync-cli`

## Usage

> [!Note]  
> You can call `sw-sync-cli help` at any time to get more information

### Authentication

1. Set up an [integration](https://docs.shopware.com/en/shopware-6-en/settings/system/integrationen?category=shopware-6-en/settings/system) inside shopware.
2. Call `sw-sync-cli auth` with the required arguments (credentials), for example:

```bash
sw-sync-cli auth -d https://your-shopware-url.com -i your-integration-id -s your-integration-secret
```

> [!WARNING]  
> This will create a `.credentials.toml` file in your current working directory.
> This file contains your credentials in plain text, you might want to remove it again after you are done syncing.

### Copying default profiles

You can copy the default profiles to your current working directory by calling:

```bash
sw-sync-cli copy-profiles
```

This will create a `profiles` folder in your current working directory with all the default profiles. You can then adapt them to your needs.

### Syncing

Call `sw-sync-cli sync` in either `-m import` or `-m export` mode, with a profile (`profile.yaml`) and data file `data.csv` as arguments, for example:

```bash
sw-sync-cli sync -m import -p profiles/product.yaml -f data.csv
sw-sync-cli sync -m export -p profiles/product.yaml -f data.csv
```

> [!Note]
> If you checked out this repository e.g. to make Rust code changes, you can also call all the above commands with `cargo run <command>`, e.g. `cargo run auth`.
> Note this way is only suggested for developers / contributors to this project.

### Profiles

Profiles are used to define the mapping between (CSV) file columns and Shopware entity fields, as well as additional configuration for the import / export.
To get started take a look at [Profiles in this repository](https://github.com/shopware/sw-sync-cli/tree/main/profiles).
The structure of a profile `.yaml` is as follows:

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
    # column type defines the data type for the internal processing of the column data
    colum_type: "string"
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

  // You can get the default language or a specific language id by their ISO code
  // Default language is used here and will return the default language id
  let default_language_id = get_default("LANGUAGE_SYSTEM");
  // For a specific language id you can use the get_language_by_iso function:
  let specific_language_id = get_language_by_iso("de-DE"); // It will return the language id for "de-DE"

  // You can also get different currencies by their ISO code
  // Default currency is used here and will return the default currency id
  let default_currency_id = get_default("CURRENCY");
  // For a specific currency id you can use the get_currency_by_iso function:
  let eur_currency_id = get_currency_by_iso("EUR"); // It will return the currency id for "EUR"
```

### Serialization / Deserialization scripts
These are optional scripts where you can run more complex serialization/deserialization logic for your specific use case. These scripts are written in the [Rhai scripting language](https://rhai.rs/book/).
The scripts are executed once per entity.

For serialization, you receive the entity object and an empty row object which you can populate. For deserialization, you receive the row object and an empty entity object which you can populate. The keys you set in the entity/row object should match the keys you defined in the mappings section. The other simple mappings are executed after these scripts.

There are some utility functions available in the scripts:
- `get_default(key: string) -> string`: Returns the default value for the given key. The following keys are available:
  - `LANGUAGE_SYSTEM`: Returns the default language id
  - `LVE_VERSION`: Returns the live version id
  - `CURRENCY`: Returns the default currency id
  - `SALES_CHANNEL_TYPE_API`: Returns the sales channel type id for the API
  - `SALES_CHANNEL_TYPE_STOREFRONT`: Returns the sales channel type id for the storefront
  - `SALES_CHANNEL_TYPE_PRODUCT_COMPARISON`: Returns the sales channel type id for the product comparison
  - `STORAGE_DATE_TIME_FORMAT`: Returns the date time format
  - `STORAGE_DATE_FORMAT`: Returns the date format
  - `CMS_PRODUCT_DETAIL_PAGE`: Returns the CMS product detail page id
- `get_language_by_iso(iso: string) -> string`: Returns the language id for the given ISO code
- `get_currency_by_iso(iso: string) -> string`: Returns the currency id for the given ISO code

## License

`sw-sync-cli` is distributed under the terms of the MIT License.  
See the [LICENSE](./LICENSE) file for details.
