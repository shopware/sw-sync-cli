//! Everything related to data transformations

pub mod script;

use crate::api::Entity;
use crate::config_file::{Mapping, Profile};
use crate::data::ScriptingEnvironment;
use anyhow::Context;
use csv::StringRecord;
use std::str::FromStr;

/// Deserialize a single row of the input (CSV) file into a json object
pub fn deserialize_row(
    headers: &StringRecord,
    row: &StringRecord,
    profile: &Profile,
    scripting_environment: &ScriptingEnvironment,
) -> anyhow::Result<Entity> {
    // Either run deserialize script or create initial empty entity object
    let mut entity = scripting_environment.run_deserialize(headers, row, profile)?;

    for mapping in &profile.mappings {
        match mapping {
            Mapping::ByPath(path_mapping) => {
                let column_index = headers
                    .iter()
                    .position(|header| header == path_mapping.file_column)
                    .with_context(|| {
                        format!(
                            "Can't find column '{}' in CSV headers",
                            path_mapping.file_column
                        )
                    })?;

                let raw_value = row
                    .get(column_index)
                    .context("failed to get column of row")?;
                let json_value = get_json_value_from_string(raw_value);

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
pub fn serialize_entity(
    entity: &Entity,
    profile: &Profile,
    scripting_environment: &ScriptingEnvironment,
) -> anyhow::Result<Vec<String>> {
    let script_row = scripting_environment.run_serialize(entity)?;
    let mut row = Vec::with_capacity(profile.mappings.len());

    for mapping in &profile.mappings {
        match mapping {
            Mapping::ByPath(path_mapping) => {
                let value = entity.get_by_path(&path_mapping.entity_path)
                    .with_context(|| format!(
                        "could not get field path '{}' specified in mapping (you might try the optional chaining operator '?.' to fallback to null), entity attributes:\n{}",
                        path_mapping.entity_path,
                        serde_json::to_string_pretty(&entity).unwrap()) // expensive for big entities
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
                    .with_context(|| {
                        format!(
                            "failed to retrieve script key '{}' of row",
                            script_mapping.key
                        )
                    })?;
                let value_str = serde_json::to_string(value)?;

                row.push(value_str);
            }
        }
    }

    Ok(row)
}

fn get_json_value_from_string(raw_input: &str) -> serde_json::Value {
    let raw_input_lowercase = raw_input.to_lowercase();
    if raw_input_lowercase == "null" || raw_input.trim().is_empty() {
        serde_json::Value::Null
    } else if raw_input_lowercase == "true" {
        serde_json::Value::Bool(true)
    } else if raw_input_lowercase == "false" {
        serde_json::Value::Bool(false)
    } else if let Ok(number) = serde_json::Number::from_str(raw_input) {
        serde_json::Value::Number(number)
    } else {
        serde_json::Value::String(raw_input.to_owned())
    }
}

trait EntityPath {
    /// Search for a value inside a json object tree by a given path.
    /// Example path `object.child.attribute`
    /// Path with null return, if not existing: `object?.child?.attribute`
    fn get_by_path(&self, path: &str) -> Option<&serde_json::Value>;

    /// Insert a value into a given path
    /// ## Invariant:
    /// Does nothing if the value is Null (to not create objects with only null values)
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
        let Some(mut value) = self.get(first_token) else {
            return if first_optional {
                Some(&serde_json::Value::Null)
            } else {
                None
            };
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
        assert!(!path.is_empty(), "empty entity_path encountered");
        if value.is_null() {
            return; // do nothing
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
    use crate::data::transform::{get_json_value_from_string, EntityPath};
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

        entity.insert_by_path("another.cousin.value", serde_json::Value::Null);
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

    #[test]
    fn test_get_json_value_from_string() {
        let value = get_json_value_from_string("null");
        assert_eq!(value, json!(null));
        let value = get_json_value_from_string("");
        assert_eq!(value, json!(null));

        let value = get_json_value_from_string("true");
        assert_eq!(value, json!(true));

        let value = get_json_value_from_string("false");
        assert_eq!(value, json!(false));

        let value = get_json_value_from_string("42.42");
        assert_eq!(value, json!(42.42));

        let value = get_json_value_from_string("my string");
        assert_eq!(value, json!("my string"));
    }
}
