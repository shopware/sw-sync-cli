# NEXT-RELEASE

- NEXT-37504 - Fixed the generation of request criteria to support correct nested associations

# v0.7.1

- Moved repository to shopware organization

# v0.7.0

- Fixed bug which caused much worse response times that happened sometimes during concurrent requests
- Slightly improved the performance and refactored much of the main processing logic to better utilize CPU cores

# v0.6.0

- Added schema / profile validation to ensure the entities + `entity_path` exists in the shop

# v0.5.0

- Added `get_default(name)` function to scripting. It allows lookup of constants like the `Shopware/src/Core/Defaults.php`
- Fixed import of "To-Many-Associations" when the value is null it will be ignored instead of added to the entity

# v0.4.0

- "To-One-Association" values are now imported correctly
  - Added profile `product_with_manufacturer.yaml` as an example
- Fixed reported request timings (they were measured wrong, longer than actual)
- Fixed `--in-flight-limit` to actually be respected (wasn't implemented correctly)
- Changed default `in_flight_limit` to `8` (from `16`) as that seemed like a better performing number
- Implemented all criteria filter types and added `product_variants.yaml`
- Removed `sync --verbose` option for now, as it wasn't implemented

# v0.3.0

- Added `associations` entry for schema (used on export only)
- Implemented proper `entity_path` resolution with optional chaining `?.` for export
- "To-One-Associations" are now exported correctly
  - The implementation for import is still missing and these fields will be ignored for now

# v0.2.0

- Added very basic `filter` entry for schema (used on export only)
- Added `sort` entry for schema (used on export only)

# v0.1.0

- Initial release
