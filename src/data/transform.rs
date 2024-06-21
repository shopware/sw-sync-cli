use crate::api::SwListEntity;
use crate::config::Mapping;
use crate::SyncContext;
use anyhow::Context;
use csv::StringRecord;

/// Deserialize a single row of the input file into a json object
pub fn deserialize_row(
    headers: &StringRecord,
    row: StringRecord,
    context: &SyncContext,
) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    let mut entity = serde_json::Map::with_capacity(context.schema.mappings.len());

    for mapping in &context.schema.mappings {
        match mapping {
            Mapping::ByPath(by_path_mapping) => {
                let column_index = headers
                    .iter()
                    .position(|header| header == by_path_mapping.file_column)
                    .context(format!(
                        "Can't find column {} in CSV headers",
                        by_path_mapping.file_column
                    ))?;

                let value = row
                    .get(column_index)
                    .context("failed to get column of row")?;
                // ToDo: properly deserialize into different json types

                entity.insert(
                    by_path_mapping.entity_path.clone(),
                    serde_json::Value::String(value.to_owned()),
                );
            }
        }
    }

    Ok(entity)
}

/// Serialize a single entity (as json object) into a single row (string columns)
pub fn serialize_entity(
    entity: SwListEntity,
    context: &SyncContext,
) -> anyhow::Result<Vec<String>> {
    let mut row = Vec::with_capacity(context.schema.mappings.len());
    for mapping in &context.schema.mappings {
        match mapping {
            Mapping::ByPath(by_path_mapping) => {
                let value = match by_path_mapping.entity_path.as_ref() {
                    "id" => &serde_json::Value::String(entity.id.to_string()),
                    path => entity.attributes.get(path).context(format!(
                        "could not get field path {} specified in mapping, entity attributes:\n{}",
                        path,
                        serde_json::to_string_pretty(&entity.attributes).unwrap()
                    ))?,
                };

                let value_str = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string(other)?,
                };

                row.push(value_str);
            }
        }
    }

    Ok(row)
}
