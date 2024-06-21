use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub base_url: String,
    pub access_key_id: String,
    pub access_key_secret: String,
}

#[derive(Debug, Deserialize)]
pub struct Schema {
    pub entity: String,
    pub mappings: Vec<Mapping>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Mapping {
    ByPath(EntityPathMapping),
}

#[derive(Debug, Deserialize)]
pub struct EntityPathMapping {
    pub file_column: String,
    pub entity_path: String,
}
