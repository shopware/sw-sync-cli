use crate::api::Entity;
use crate::config_file::{EntityPathMapping, Mapping};

/// Validate paths for entity
pub fn validate_paths_for_entity(
    entity: &str,
    mappings: &Vec<Mapping>,
    api_schema: &Entity,
) -> anyhow::Result<()> {
    // if entity name is not set in api_schema throw an exception
    if !api_schema.contains_key(entity) {
        anyhow::bail!("Entity {} not found in API schema", entity);
    }

    for entry in mappings {
        let path_mapping = match entry {
            Mapping::ByPath(path_mapping) => path_mapping,
            Mapping::ByScript(_) => continue,
        };

        let path = path_mapping.entity_path.split('.').collect::<Vec<_>>();
        let root_path = path[0];

        // if path ends with ? remove it
        let root_path = root_path.trim_end_matches('?');

        let Some(root_property) = api_schema
            .get(entity)
            .and_then(|x| x.get("properties"))
            .and_then(|x| x.get(root_path))
            .and_then(|x| x.as_object())
        else {
            anyhow::bail!("Entity {} does not have a field {}", entity, root_path);
        };

        // if path has only one part it should be a simple field
        if path.len() == 1 {
            continue;
        }

        // if its multiple parts it should be an association
        if root_property["type"].as_str().unwrap() != "association" {
            anyhow::bail!("Field {} in {} is not an association", root_path, entity);
        }

        let entity_name = root_property["entity"].as_str().unwrap();
        let path = path[1..].join(".");

        // create a new mapping with the new path
        let mapping = Mapping::ByPath(EntityPathMapping {
            file_column: path_mapping.file_column.clone(),
            entity_path: path,
            column_type: path_mapping.column_type.clone(),
        });

        // validate the new mapping
        validate_paths_for_entity(entity_name, &vec![mapping], api_schema)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config_file::{EntityPathMapping, Mapping};
    use serde_json::json;

    #[test]
    fn validate_non_existent_entity() {
        let entity = "nonexistent";
        let mapping = vec![Mapping::ByPath(EntityPathMapping {
            file_column: "manufacturer id".to_string(),
            entity_path: "manufacturerId".to_string(),
            column_type: None,
        })];
        let api_schema = json!({
            "product": {
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(
            entity,
            &mapping,
            api_schema.as_object().unwrap(),
        );

        assert!(result.is_err_and(|x| x
            .to_string()
            .contains("Entity nonexistent not found in API schema")));
    }

    #[test]
    fn validate_non_existent_simple_path() {
        let entity = "product";
        let mapping = vec![Mapping::ByPath(EntityPathMapping {
            file_column: "manufacturer id".to_string(),
            entity_path: "manufacturerId".to_string(),
            column_type: None,
        })];
        let api_schema = json!({
            "product": {
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(
            entity,
            &mapping,
            api_schema.as_object().unwrap(),
        );

        assert!(result.is_err_and(|x| x
            .to_string()
            .contains("Entity product does not have a field manufacturerId")));
    }

    #[test]
    fn validate_existing_simple_path() {
        let entity = "product";
        let mapping = vec![Mapping::ByPath(EntityPathMapping {
            file_column: "manufacturer id".to_string(),
            entity_path: "manufacturerId".to_string(),
            column_type: None,
        })];
        let api_schema = json!({
            "product": {
                "entity": "product",
                "properties": {
                    "manufacturerId": {
                        "type": "uuid"
                    }
                }
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(
            entity,
            &mapping,
            api_schema.as_object().unwrap(),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn validate_non_existent_association() {
        let entity = "product";
        let mapping = vec![Mapping::ByPath(EntityPathMapping {
            file_column: "manufacturer name".to_string(),
            entity_path: "manufacturer.name".to_string(),
            column_type: None,
        })];
        let api_schema = json!({
            "product": {
                "entity": "product",
                "properties": {
                    "manufacturer": {
                        "type": "string",
                    }
                }
            },
        });

        let result = crate::data::validate::validate_paths_for_entity(
            entity,
            &mapping,
            api_schema.as_object().unwrap(),
        );

        assert!(result.is_err_and(|x| x
            .to_string()
            .contains("Field manufacturer in product is not an association")));
    }

    #[test]
    fn validate_existing_association() {
        let entity = "product";
        let mapping = vec![Mapping::ByPath(EntityPathMapping {
            file_column: "manufacturer name".to_string(),
            entity_path: "manufacturer.name".to_string(),
            column_type: None,
        })];
        let api_schema = json!({
            "product": {
                "entity": "product",
                "properties": {
                    "manufacturer": {
                        "type": "association",
                        "entity": "product_manufacturer"
                    }
                }
            },
            "product_manufacturer": {
                "entity": "product_manufacturer",
                "properties": {
                    "name": {
                        "type": "string"
                    }
                }
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(
            entity,
            &mapping,
            api_schema.as_object().unwrap(),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn validate_valid_optional_value() {
        let entity = "product";
        let mapping = vec![Mapping::ByPath(EntityPathMapping {
            file_column: "manufacturer name".to_string(),
            entity_path: "manufacturer?.name".to_string(),
            column_type: None,
        })];
        let api_schema = json!({
            "product": {
                "entity": "product",
                "properties": {
                    "manufacturer": {
                        "type": "association",
                        "entity": "product_manufacturer"
                    }
                }
            },
            "product_manufacturer": {
                "entity": "product_manufacturer",
                "properties": {
                    "name": {
                        "type": "string"
                    }
                }
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(
            entity,
            &mapping,
            api_schema.as_object().unwrap(),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn validate_invalid_optional_value() {
        let entity = "product";
        let mapping = vec![Mapping::ByPath(EntityPathMapping {
            file_column: "manufacturer name".to_string(),
            entity_path: "manufacturer?.name".to_string(),
            column_type: None,
        })];
        let api_schema = json!({
            "product": {
                "entity": "product",
                "properties": {
                    "manufacturer": {
                        "type": "association",
                        "entity": "product_manufacturer"
                    }
                }
            },
            "product_manufacturer": {
                "entity": "product_manufacturer",
                "properties": {
                    "id": {
                        "type": "uuid"
                    }
                }
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(
            entity,
            &mapping,
            api_schema.as_object().unwrap(),
        );

        assert!(result.is_err_and(|x| x
            .to_string()
            .contains("Entity product_manufacturer does not have a field name")));
    }

    #[test]
    fn validate_valid_nested_association() {
        let entity = "product";
        let mapping = vec![Mapping::ByPath(EntityPathMapping {
            file_column: "tax country".to_string(),
            entity_path: "tax.country.name".to_string(),
            column_type: None,
        })];
        let api_schema = json!({
            "product": {
                "entity": "product",
                "properties": {
                    "tax": {
                        "type": "association",
                        "entity": "tax"
                    }
                }
            },
            "tax": {
                "entity": "tax",
                "properties": {
                    "country": {
                        "type": "association",
                        "entity": "country"
                    }
                }
            },
            "country": {
                "entity": "country",
                "properties": {
                    "name": {
                        "type": "string",
                    }
                }
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(
            entity,
            &mapping,
            api_schema.as_object().unwrap(),
        );

        assert!(result.is_ok());
    }
}
