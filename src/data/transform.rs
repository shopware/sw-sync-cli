use crate::api::Entity;
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
                    "Can't find column '{}' in CSV headers",
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
                        "Can't find column '{}' in CSV headers",
                        path_mapping.file_column
                    ))?;

                let raw_value = row
                    .get(column_index)
                    .context("failed to get column of row")?;
                let raw_value_lowercase = raw_value.to_lowercase();

                let json_value = if raw_value_lowercase == "null" || raw_value.trim().is_empty() {
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

                entity.insert_by_path(&path_mapping.entity_path, json_value);
            }
            Mapping::ByScript(_script_mapping) => {
                // nothing to do here, the script already executed beforehand
            }
        }
    }

    Ok(entity)
}

/// Serialize a single entity (as json object) into a single row (string columns)
pub fn serialize_entity(entity: Entity, context: &SyncContext) -> anyhow::Result<Vec<String>> {
    let script_row = if let Some(serialize) = &context.scripting_environment.serialize {
        let engine = &context.scripting_environment.engine;

        let mut scope = Scope::new();
        let script_entity = rhai::serde::to_dynamic(&entity)?;
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
                let value = entity.get_by_path(&path_mapping.entity_path)
                    .context(format!(
                        "could not get field path '{}' specified in mapping (you might try the optional chaining operator '?.' to fallback to null), entity attributes:\n{}",
                        path_mapping.entity_path,
                        serde_json::to_string_pretty(&entity).unwrap())
                    )?;

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
                        "failed to retrieve script key '{}' of row",
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

trait EntityPath {
    /// Search for a value inside a json object tree by a given path.
    /// Example path `object.child.attribute`
    /// Path with null return, if not existing: `object?.child?.attribute`
    fn get_by_path(&self, path: &str) -> Option<&serde_json::Value>;

    /// Insert a value into a given path
    fn insert_by_path(&mut self, path: &str, value: serde_json::Value);
}

impl EntityPath for Entity {
    // based on the pointer implementation in serde_json::Value
    fn get_by_path(&self, path: &str) -> Option<&serde_json::Value> {
        if path.is_empty() {
            return None;
        }

        let tokens = path.split('.');
        let mut optional_chain = tokens.clone().map(|token| token.ends_with('?'));
        let mut tokens = tokens.map(|t| t.trim_end_matches('?'));

        // initial access happens on map
        let first_token = tokens.next()?;
        let first_optional = optional_chain.next()?;
        let mut value = match self.get(first_token) {
            Some(v) => v,
            None => {
                if first_optional {
                    return Some(&serde_json::Value::Null);
                } else {
                    return None;
                }
            }
        };

        // the question mark refers to the current token and allows it to be undefined
        // https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Operators/Optional_chaining
        for (token, is_optional) in tokens.zip(optional_chain) {
            value = match value {
                serde_json::Value::Object(map) => match map.get(token) {
                    Some(v) => v,
                    None => {
                        return if is_optional {
                            Some(&serde_json::Value::Null)
                        } else {
                            None
                        }
                    }
                },
                serde_json::Value::Null => {
                    return Some(&serde_json::Value::Null);
                }
                _ => {
                    return None;
                }
            }
        }

        Some(value)
    }

    fn insert_by_path(&mut self, path: &str, value: serde_json::Value) {
        if path.is_empty() {
            panic!("empty entity_path encountered");
        }

        let mut tokens = path.split('.').map(|t| t.trim_end_matches('?')).peekable();

        let first_token = tokens.next().expect("has a value because non empty");
        let pointer = self.entry(first_token).or_insert_with(|| {
            if tokens.peek().is_none() {
                value.clone()
            } else {
                let child = Entity::with_capacity(1);
                serde_json::Value::Object(child)
            }
        });
        if tokens.peek().is_none() {
            *pointer = value;
            return;
        }

        let mut pointer = pointer
            .as_object_mut()
            .expect("insert_by_path lead to non object");
        while let Some(token) = tokens.next() {
            if tokens.peek().is_none() {
                // simply insert the value
                pointer.insert(token.to_string(), value);
                return;
            }

            pointer = pointer
                .entry(token)
                .or_insert_with(|| {
                    let child = Entity::with_capacity(1);
                    serde_json::Value::Object(child)
                })
                .as_object_mut()
                .expect("insert_by_path lead to non object");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::data::transform::EntityPath;
    use serde_json::{json, Number, Value};

    #[test]
    fn test_get_by_path() {
        let child = json!({
            "attribute": 42,
            "hello": null
        });
        let entity = json!({
            "child": child,
            "fizz": "buzz",
            "hello": null,
        });

        let entity = match entity {
            Value::Object(map) => map,
            _ => unreachable!(),
        };

        assert_eq!(
            entity.get_by_path("fizz"),
            Some(&Value::String("buzz".into()))
        );
        assert_eq!(entity.get_by_path("child"), Some(&child));
        assert_eq!(entity.get_by_path("bar"), None);
        assert_eq!(
            entity.get_by_path("child.attribute"),
            Some(&Value::Number(Number::from(42)))
        );
        assert_eq!(entity.get_by_path("child.bar"), None);
        assert_eq!(entity.get_by_path("child.fizz.bar"), None);

        // optional chaining
        assert_eq!(entity.get_by_path("child?.bar"), None);
        assert_eq!(entity.get_by_path("child?.bar?.fizz"), Some(&Value::Null));
        assert_eq!(entity.get_by_path("child?.attribute?.fizz"), None); // invalid access (attribute is not a map)
        assert_eq!(entity.get_by_path("hello?.bar"), Some(&Value::Null));
        assert_eq!(entity.get_by_path("child.hello"), Some(&Value::Null));
        assert_eq!(entity.get_by_path("child.hello?.bar"), Some(&Value::Null));
    }

    #[test]
    fn test_insert_by_path() {
        let entity = json!({
            "fiz": "buz"
        });
        let mut entity = match entity {
            Value::Object(map) => map,
            _ => unreachable!(),
        };

        entity.insert_by_path("child.bar", json!("hello"));
        assert_eq!(
            Value::Object(entity.clone()),
            json!({
                "fiz": "buz",
                "child": {
                    "bar": "hello",
                },
            })
        );

        entity.insert_by_path("another.nested.child.value", json!(42));
        assert_eq!(
            Value::Object(entity.clone()),
            json!({
                "fiz": "buz",
                "child": {
                    "bar": "hello",
                },
                "another": {
                    "nested": {
                        "child": {
                            "value": 42,
                        },
                    },
                },
            })
        );

        entity.insert_by_path("fiz", json!(42));
        assert_eq!(
            Value::Object(entity.clone()),
            json!({
                "fiz": 42,
                "child": {
                    "bar": "hello",
                },
                "another": {
                    "nested": {
                        "child": {
                            "value": 42,
                        },
                    },
                },
            })
        );

        entity.insert_by_path("child.bar", json!("buz"));
        assert_eq!(
            Value::Object(entity.clone()),
            json!({
                "fiz": 42,
                "child": {
                    "bar": "buz",
                },
                "another": {
                    "nested": {
                        "child": {
                            "value": 42,
                        },
                    },
                },
            })
        );

        entity.insert_by_path("child.hello", json!("world"));
        assert_eq!(
            Value::Object(entity.clone()),
            json!({
                "fiz": 42,
                "child": {
                    "bar": "buz",
                    "hello": "world",
                },
                "another": {
                    "nested": {
                        "child": {
                            "value": 42,
                        },
                    },
                },
            })
        );

        entity.insert_by_path("another.nested.sibling", json!({"type": "cousin"}));
        assert_eq!(
            Value::Object(entity.clone()),
            json!({
                "fiz": 42,
                "child": {
                    "bar": "buz",
                    "hello": "world",
                },
                "another": {
                    "nested": {
                        "child": {
                            "value": 42,
                        },
                        "sibling": {
                            "type": "cousin",
                        },
                    },
                },
            })
        );

        entity.insert_by_path("another.nested", json!("replaced"));
        assert_eq!(
            Value::Object(entity.clone()),
            json!({
                "fiz": 42,
                "child": {
                    "bar": "buz",
                    "hello": "world",
                },
                "another": {
                    "nested": "replaced"
                },
            })
        );
    }
}
