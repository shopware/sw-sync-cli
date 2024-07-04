use crate::config::Mapping;

/// Validate paths for entity
pub fn validate_paths_for_entity(
    entity: &str,
    mappings: &Vec<Mapping>,
    api_schema: &serde_json::Map<String, serde_json::Value>,
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

        // if path starts with ? its optional
        if path[0].ends_with('?') {
            continue;
        }

        // check if first path element is set in the entity object in schema
        if !api_schema[entity]["properties"][path[0]].is_object() {
            anyhow::bail!("Entity {} does not have a field {}", entity, path[0]);
        }

        let root_path = api_schema[entity]["properties"][path[0]].as_object().unwrap();

        // if there is only one path element
        if path.len() == 1 {
            let field_type = root_path["type"].as_str().unwrap();

            // check if type matches the type defined in the schema
            if field_type != path_mapping.field_type {
                anyhow::bail!("Type {} does not match schema type {} for {} in {}", path_mapping.field_type, field_type, path[0], entity);
            }
        } else {
            // if its multiple parts it should be an association
            if root_path["type"].as_str().unwrap() != "association" {
                anyhow::bail!("Field {} in {} is not an association", path[0], entity);
            }

            let entity_name = root_path["entity"].as_str().unwrap();
            let path = path[1..].join(".");

            // create a new mapping with the new path
            let mapping = Mapping::ByPath(crate::config::EntityPathMapping {
                file_column: path_mapping.file_column.clone(),
                entity_path: path,
                field_type: path_mapping.field_type.clone(),
            });

            // validate the new mapping
            validate_paths_for_entity(entity_name, &vec![mapping], api_schema)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn validate_non_existent_entity() {
        let entity = "nonexistent";
        let mapping = vec![crate::config::Mapping::ByPath(crate::config::EntityPathMapping {
            file_column: "manufacturer id".to_string(),
            entity_path: "manufacturerId".to_string(),
            field_type: "uuid".to_string(),
        })];
        let api_schema = json!({
            "product": {
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(entity, &mapping, api_schema.as_object().unwrap());

        assert!(result.is_err_and(|x| x.to_string().contains("Entity nonexistent not found in API schema")));
    }

    #[test]
    fn validate_non_existent_simple_path() {
        let entity = "product";
        let mapping = vec![crate::config::Mapping::ByPath(crate::config::EntityPathMapping {
            file_column: "manufacturer id".to_string(),
            entity_path: "manufacturerId".to_string(),
            field_type: "uuid".to_string(),
        })];
        let api_schema = json!({
            "product": {
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(entity, &mapping, api_schema.as_object().unwrap());

        assert!(result.is_err_and(|x| x.to_string().contains("Entity product does not have a field manufacturerId")));
    }

    #[test]
    fn validate_optional_simple_path() {
        let entity = "product";
        let mapping = vec![crate::config::Mapping::ByPath(crate::config::EntityPathMapping {
            file_column: "manufacturer id".to_string(),
            entity_path: "manufacturerId".to_string(),
            field_type: "uuid".to_string(),
        })];
        let api_schema = json!({
            "product": {
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(entity, &mapping, api_schema.as_object().unwrap());

        assert!(result.is_err_and(|x| x.to_string().contains("Entity product does not have a field manufacturerId")));
    }

    #[test]
    fn validate_existing_simple_path_but_type_mismatch() {
        let entity = "product";
        let mapping = vec![crate::config::Mapping::ByPath(crate::config::EntityPathMapping {
            file_column: "manufacturer id".to_string(),
            entity_path: "manufacturerId".to_string(),
            field_type: "string".to_string(),
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

        let result = crate::data::validate::validate_paths_for_entity(entity, &mapping, api_schema.as_object().unwrap());

        assert!(result.is_err_and(|x| x.to_string().contains("Type string does not match schema type uuid for manufacturerId in product")));
    }

    #[test]
    fn validate_existing_simple_path() {
        let entity = "product";
        let mapping = vec![crate::config::Mapping::ByPath(crate::config::EntityPathMapping {
            file_column: "manufacturer id".to_string(),
            entity_path: "manufacturerId".to_string(),
            field_type: "uuid".to_string(),
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

        let result = crate::data::validate::validate_paths_for_entity(entity, &mapping, api_schema.as_object().unwrap());

        assert!(result.is_ok());
    }

    #[test]
    fn validate_non_existent_association() {
        let entity = "product";
        let mapping = vec![crate::config::Mapping::ByPath(crate::config::EntityPathMapping {
            file_column: "manufacturer name".to_string(),
            entity_path: "manufacturer.name".to_string(),
            field_type: "string".to_string(),
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

        let result = crate::data::validate::validate_paths_for_entity(entity, &mapping, api_schema.as_object().unwrap());

        assert!(result.is_err_and(|x| x.to_string().contains("Field manufacturer in product is not an association")));
    }

    #[test]
    fn validate_existing_association() {
        let entity = "product";
        let mapping = vec![crate::config::Mapping::ByPath(crate::config::EntityPathMapping {
            file_column: "manufacturer name".to_string(),
            entity_path: "manufacturer.name".to_string(),
            field_type: "string".to_string(),
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

        let result = crate::data::validate::validate_paths_for_entity(entity, &mapping, api_schema.as_object().unwrap());

        assert!(result.is_ok());
    }

    #[test]
    fn validate_existing_association_but_type_mismatch() {
        let entity = "product";
        let mapping = vec![crate::config::Mapping::ByPath(crate::config::EntityPathMapping {
            file_column: "manufacturer id".to_string(),
            entity_path: "manufacturer.id".to_string(),
            field_type: "string".to_string(),
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

        let result = crate::data::validate::validate_paths_for_entity(entity, &mapping, api_schema.as_object().unwrap());

        assert!(result.is_err_and(|x| x.to_string().contains("Type string does not match schema type uuid for id in product_manufacturer")));
    }

    #[test]
    fn validate_optional_association() {
        let entity = "product";
        let mapping = vec![crate::config::Mapping::ByPath(crate::config::EntityPathMapping {
            file_column: "manufacturer name".to_string(),
            entity_path: "manufacturer?.name".to_string(),
            field_type: "string".to_string(),
        })];
        let api_schema = json!({
            "product": {
                "entity": "product",
                "properties": {}
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(entity, &mapping, api_schema.as_object().unwrap());

        assert!(result.is_ok());
    }

    #[test]
    fn validate_valid_nested_association() {
        let entity = "product";
        let mapping = vec![crate::config::Mapping::ByPath(crate::config::EntityPathMapping {
            file_column: "tax country".to_string(),
            entity_path: "tax.country.name".to_string(),
            field_type: "string".to_string(),
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

        let result = crate::data::validate::validate_paths_for_entity(entity, &mapping, api_schema.as_object().unwrap());

        assert!(result.is_ok());
    }

    #[test]
    fn validate_invalid_nested_association() {
        let entity = "product";
        let mapping = vec![crate::config::Mapping::ByPath(crate::config::EntityPathMapping {
            file_column: "tax country".to_string(),
            entity_path: "tax.country.id".to_string(),
            field_type: "string".to_string(),
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
                    "id": {
                        "type": "uuid",
                    }
                }
            }
        });

        let result = crate::data::validate::validate_paths_for_entity(entity, &mapping, api_schema.as_object().unwrap());

        assert!(result.is_err_and(|x| x.to_string().contains("Type string does not match schema type uuid for id in country")));
    }
}