use crate::api::SwListEntity;
use crate::config::Mapping;
use crate::SyncContext;
use anyhow::Context;
use csv::StringRecord;
use rhai::packages::{BasicArrayPackage, CorePackage, MoreStringPackage, Package};
use rhai::{Engine, Position, Scope, AST};
use std::str::FromStr;

/// Deserialize a single row of the input file into a json object
pub fn deserialize_row(
    headers: &StringRecord,
    row: StringRecord,
    context: &SyncContext,
) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    let mut entity = if let Some(deserialize) = &context.scripting_environment.deserialize {
        let engine = &context.scripting_environment.engine;

        let mut scope = Scope::new();

        // build row object
        let mut script_row = rhai::Map::new();
        let script_mappings = context.schema.mappings.iter().filter_map(|m| match m {
            Mapping::ByScript(s) => Some(s),
            _ => None,
        });
        for mapping in script_mappings {
            let column_index = headers
                .iter()
                .position(|h| h == mapping.file_column)
                .context(format!(
                    "Can't find column {} in CSV headers",
                    mapping.file_column
                ))?;

            let value = row
                .get(column_index)
                .context("failed to get column of row")?;

            script_row.insert(mapping.key.as_str().into(), value.into());
        }

        scope.push_constant("row", script_row);
        let entity_dynamic = rhai::Map::new();
        scope.push("entity", entity_dynamic);

        engine.run_ast_with_scope(&mut scope, deserialize)?;

        let row_result: rhai::Map = scope
            .get_value("entity")
            .expect("row should exist in script scope");
        let mut entity_after_script = serde_json::Map::with_capacity(context.schema.mappings.len());
        for (key, value) in row_result {
            let json_value: serde_json::Value = rhai::serde::from_dynamic(&value)?;
            entity_after_script.insert(key.to_string(), json_value);
        }

        entity_after_script
    } else {
        serde_json::Map::with_capacity(context.schema.mappings.len())
    };

    for mapping in &context.schema.mappings {
        match mapping {
            Mapping::ByPath(path_mapping) => {
                let column_index = headers
                    .iter()
                    .position(|header| header == path_mapping.file_column)
                    .context(format!(
                        "Can't find column {} in CSV headers",
                        path_mapping.file_column
                    ))?;

                let raw_value = row
                    .get(column_index)
                    .context("failed to get column of row")?;
                let raw_value_lowercase = raw_value.to_lowercase();

                let json_value = if raw_value_lowercase == "null" {
                    serde_json::Value::Null
                } else if raw_value_lowercase == "true" {
                    serde_json::Value::Bool(true)
                } else if raw_value_lowercase == "false" {
                    serde_json::Value::Bool(false)
                } else if let Ok(number) = serde_json::Number::from_str(raw_value) {
                    serde_json::Value::Number(number)
                } else {
                    serde_json::Value::String(raw_value.to_owned())
                };

                entity.insert(path_mapping.entity_path.clone(), json_value);
            }
            Mapping::ByScript(_script_mapping) => {
                // nothing to do here, the script already executed beforehand
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
    let script_row = if let Some(serialize) = &context.scripting_environment.serialize {
        let engine = &context.scripting_environment.engine;

        let mut scope = Scope::new();
        let script_entity = rhai::serde::to_dynamic(&entity.attributes)?;
        scope.push_dynamic("entity", script_entity);
        let row_dynamic = rhai::Map::new();
        scope.push("row", row_dynamic);

        engine.run_ast_with_scope(&mut scope, serialize)?;

        let row_result: rhai::Map = scope
            .get_value("row")
            .expect("row should exist in script scope");
        row_result
    } else {
        rhai::Map::new()
    };

    let mut row = Vec::with_capacity(context.schema.mappings.len());
    for mapping in &context.schema.mappings {
        match mapping {
            Mapping::ByPath(path_mapping) => {
                let value = match path_mapping.entity_path.as_ref() {
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
            Mapping::ByScript(script_mapping) => {
                let value = script_row
                    .get(script_mapping.key.as_str())
                    .context(format!(
                        "failed to retrieve script key {} of row",
                        script_mapping.key
                    ))?;
                let value_str = serde_json::to_string(value)?;

                row.push(value_str);
            }
        }
    }

    Ok(row)
}

#[derive(Debug)]
pub struct ScriptingEnvironment {
    engine: Engine,
    serialize: Option<AST>,
    deserialize: Option<AST>,
}

pub fn prepare_scripting_environment(
    raw_serialize_script: &str,
    raw_deserialize_script: &str,
) -> anyhow::Result<ScriptingEnvironment> {
    let engine = get_base_engine();
    let serialize_ast = if !raw_serialize_script.is_empty() {
        let ast = engine
            .compile(raw_serialize_script)
            .context("serialize_script compilation failed")?;
        Some(ast)
    } else {
        None
    };
    let deserialize_ast = if !raw_deserialize_script.is_empty() {
        let ast = engine
            .compile(raw_deserialize_script)
            .context("serialize_script compilation failed")?;
        Some(ast)
    } else {
        None
    };

    Ok(ScriptingEnvironment {
        engine,
        serialize: serialize_ast,
        deserialize: deserialize_ast,
    })
}

fn get_base_engine() -> Engine {
    let mut engine = Engine::new_raw();
    // Default print/debug implementations
    engine.on_print(|text| println!("{text}"));
    engine.on_debug(|text, source, pos| match (source, pos) {
        (Some(source), Position::NONE) => println!("{source} | {text}"),
        (Some(source), pos) => println!("{source} @ {pos:?} | {text}"),
        (None, Position::NONE) => println!("{text}"),
        (None, pos) => println!("{pos:?} | {text}"),
    });

    let core_package = CorePackage::new();
    core_package.register_into_engine(&mut engine);
    let string_package = MoreStringPackage::new();
    string_package.register_into_engine(&mut engine);
    let array_package = BasicArrayPackage::new();
    array_package.register_into_engine(&mut engine);

    // ToDo: add custom utility functions to engine
    // Some reference implementations below
    /*
    engine.register_type::<Uuid>();
    engine.register_fn("uuid", scripts::uuid);
    engine.register_fn("uuidFromStr", scripts::uuid_from_str);

    engine.register_type::<scripts::Mapper>();
    engine.register_fn("map", scripts::Mapper::map);
    engine.register_fn("get", scripts::Mapper::get);

    engine.register_type::<scripts::DB>();
    engine.register_fn("fetchFirst", scripts::DB::fetch_first);
     */

    engine
}
