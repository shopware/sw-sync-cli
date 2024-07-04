use crate::api::filter::{CriteriaFilter, CriteriaSorting};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub base_url: String,
    pub access_key_id: String,
    pub access_key_secret: String,
}

#[derive(Debug, Deserialize)]
pub struct Schema {
    pub entity: String,
    #[serde(default = "Vec::new")]
    pub filter: Vec<CriteriaFilter>,
    #[serde(default = "Vec::new")]
    pub sort: Vec<CriteriaSorting>,
    #[serde(default = "HashSet::new")]
    pub associations: HashSet<String>,
    pub mappings: Vec<Mapping>,
    #[serde(default = "String::new")]
    pub serialize_script: String,
    #[serde(default = "String::new")]
    pub deserialize_script: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Mapping {
    ByPath(EntityPathMapping),
    ByScript(EntityScriptMapping),
}

impl Mapping {
    pub fn get_file_column(&self) -> &str {
        match self {
            Mapping::ByPath(m) => &m.file_column,
            Mapping::ByScript(m) => &m.file_column,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntityPathMapping {
    pub file_column: String,
    pub entity_path: String,
    pub field_type: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntityScriptMapping {
    pub file_column: String,
    /// used as an identifier inside the script
    pub key: String,
}
