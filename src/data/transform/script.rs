//! Everything scripting related

use crate::api::Entity;
use crate::config_file::Mapping;
use crate::SyncContext;
use anyhow::Context;
use csv::StringRecord;
use rhai::packages::{BasicArrayPackage, CorePackage, MoreStringPackage, Package};
use rhai::{Engine, OptimizationLevel, Position, Scope, AST};

#[derive(Debug)]
pub struct ScriptingEnvironment {
    pub engine: Engine,
    pub serialize: Option<AST>,
    pub deserialize: Option<AST>,
}

impl ScriptingEnvironment {
    /// Just returns a default value if there is no script
    pub fn run_deserialize(
        &self,
        headers: &StringRecord,
        row: &StringRecord,
        context: &SyncContext,
    ) -> anyhow::Result<Entity> {
        let Some(deserialize_script) = &self.deserialize else {
            return Ok(Entity::with_capacity(context.schema.mappings.len()));
        };

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
                .with_context(|| {
                    format!("Can't find column '{}' in CSV headers", mapping.file_column)
                })?;

            let value = row
                .get(column_index)
                .context("failed to get column of row")?;

            script_row.insert(mapping.key.as_str().into(), value.into());
        }

        // run the script
        let mut scope = Scope::new();
        scope.push_constant("row", script_row);
        let entity_dynamic = rhai::Map::new();
        scope.push("entity", entity_dynamic);

        self.engine
            .run_ast_with_scope(&mut scope, deserialize_script)?;

        // get the entity out of the script
        let row_result: rhai::Map = scope
            .get_value("entity")
            .expect("row should exist in script scope");
        let mut entity_after_script = Entity::with_capacity(context.schema.mappings.len());
        for (key, value) in row_result {
            let json_value: serde_json::Value = rhai::serde::from_dynamic(&value)?;
            entity_after_script.insert(key.to_string(), json_value);
        }

        Ok(entity_after_script)
    }

    /// Just returns a default value if there is no script
    pub fn run_serialize(&self, entity: &Entity) -> anyhow::Result<rhai::Map> {
        let Some(serialize_script) = &self.serialize else {
            return Ok(rhai::Map::new());
        };

        let mut scope = Scope::new();

        // this is potentially expensive for big entities!
        // we might only want to pass some data into the script...
        let script_entity = rhai::serde::to_dynamic(entity)?;

        scope.push_dynamic("entity", script_entity);
        let row_dynamic = rhai::Map::new();
        scope.push("row", row_dynamic);

        self.engine
            .run_ast_with_scope(&mut scope, serialize_script)?;

        let row_result: rhai::Map = scope
            .get_value("row")
            .expect("row should exist in script scope");
        Ok(row_result)
    }
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
    engine.set_optimization_level(OptimizationLevel::Full);

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
    engine.register_fn("get_default", inside_script::get_default);

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

/// Utilities for inside scripts
///
/// Important, don't use the type `String` as function parameters, see
/// https://rhai.rs/book/rust/strings.html
mod inside_script {
    use rhai::ImmutableString;

    /// Imitate
    /// https://github.com/shopware/shopware/blob/03cfe8cca937e6e45c9c3e15821d1449dfd01d82/src/Core/Defaults.php
    pub fn get_default(name: &str) -> ImmutableString {
        match name {
            "LANGUAGE_SYSTEM" => "2fbb5fe2e29a4d70aa5854ce7ce3e20b".into(),
            "LIVE_VERSION" => "0fa91ce3e96a4bc2be4bd9ce752c3425".into(),
            "CURRENCY" => "b7d2554b0ce847cd82f3ac9bd1c0dfca".into(),
            "SALES_CHANNEL_TYPE_API" => "f183ee5650cf4bdb8a774337575067a6".into(),
            "SALES_CHANNEL_TYPE_STOREFRONT" => "8a243080f92e4c719546314b577cf82b".into(),
            "SALES_CHANNEL_TYPE_PRODUCT_COMPARISON" => "ed535e5722134ac1aa6524f73e26881b".into(),
            "STORAGE_DATE_TIME_FORMAT" => "Y-m-d H:i:s.v".into(),
            "STORAGE_DATE_FORMAT" => "Y-m-d".into(),
            "CMS_PRODUCT_DETAIL_PAGE" => "7a6d253a67204037966f42b0119704d5".into(),
            n => panic!(
                "get_default called with '{}' but there is no such definition. Have a look into Shopware/src/Core/Defaults.php. Available constants: {:?}",
                n,
                vec![
                    "LANGUAGE_SYSTEM",
                    "LIVE_VERSION",
                    "CURRENCY",
                    "SALES_CHANNEL_TYPE_API",
                    "SALES_CHANNEL_TYPE_STOREFRONT",
                    "SALES_CHANNEL_TYPE_PRODUCT_COMPARISON",
                    "STORAGE_DATE_TIME_FORMAT",
                    "STORAGE_DATE_FORMAT",
                    "CMS_PRODUCT_DETAIL_PAGE",
                ]
            )
        }
    }
}
