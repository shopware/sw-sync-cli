//! Definitions for the `profile.yaml` and `.credentials.toml` files
//!
//! Allows deserialization into a proper typed structure from these files
//! or also write these typed structures to a file (in case of `.credentials.toml`)
//!
//! Utilizes <https://serde.rs/>

use crate::api::filter::{CriteriaFilter, CriteriaSorting};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

pub const DEFAULT_PROFILES: &[(&str, &str)] = &[
    (
        "default_advanced_price.yaml",
        include_str!("../profiles/default_advanced_price.yaml"),
    ),
    (
        "default_category.yaml",
        include_str!("../profiles/default_category.yaml"),
    ),
    (
        "default_cross_selling.yaml",
        include_str!("../profiles/default_cross_selling.yaml"),
    ),
    (
        "default_customer.yaml",
        include_str!("../profiles/default_customer.yaml"),
    ),
    (
        "default_media.yaml",
        include_str!("../profiles/default_media.yaml"),
    ),
    (
        "default_newsletter_recipient.yaml",
        include_str!("../profiles/default_newsletter_recipient.yaml"),
    ),
    (
        "default_order.yaml",
        include_str!("../profiles/default_order.yaml"),
    ),
    (
        "default_product.yaml",
        include_str!("../profiles/default_product.yaml"),
    ),
    (
        "default_product_variants.yaml",
        include_str!("../profiles/default_product_variants.yaml"),
    ),
    (
        "default_promotion_code.yaml",
        include_str!("../profiles/default_promotion_code.yaml"),
    ),
    (
        "default_promotion_discount.yaml",
        include_str!("../profiles/default_promotion_discount.yaml"),
    ),
    (
        "default_property.yaml",
        include_str!("../profiles/default_property.yaml"),
    ),
    (
        "default_variant_configuration.yaml",
        include_str!("../profiles/default_variant_configuration.yaml"),
    ),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub base_url: String,
    pub access_key_id: String,
    pub access_key_secret: String,
}

impl Credentials {
    pub async fn read_credentials() -> anyhow::Result<Self> {
        let serialized_credentials = tokio::fs::read_to_string("./.credentials.toml")
            .await
            .context("No .credentials.toml found. Call command auth first.")?;

        let credentials: Self = toml::from_str(&serialized_credentials)?;
        Ok(credentials)
    }
}

#[derive(Debug, Default, Eq, PartialEq, Deserialize)]
pub struct Profile {
    pub entity: String,

    #[serde(default = "Vec::new")]
    pub filter: Vec<CriteriaFilter>,

    #[serde(default = "Vec::new")]
    pub sort: Vec<CriteriaSorting>,

    /// Are unique thanks to `HashSet`
    #[serde(default = "HashSet::new")]
    pub associations: HashSet<String>,

    pub mappings: Vec<Mapping>,

    #[serde(default = "String::new")]
    pub serialize_script: String,

    #[serde(default = "String::new")]
    pub deserialize_script: String,
}

impl Profile {
    pub async fn read_profile(profile_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let serialized_profile = tokio::fs::read_to_string(profile_path)
            .await
            .context("Provided profile file not found")?;

        let profile: Self = serde_yaml::from_str(&serialized_profile)?;
        Ok(profile)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
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

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize)]
pub struct EntityPathMapping {
    pub file_column: String,
    pub entity_path: String,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize)]
pub struct EntityScriptMapping {
    pub file_column: String,
    /// used as an identifier inside the script
    pub key: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Entity;
    use crate::data::validate_paths_for_entity;

    #[test]
    fn all_default_profiles_should_be_included() {
        let repository_profile_files =
            std::fs::read_dir("./profiles").expect("failed to read dir ./profiles");

        for repo_entry in repository_profile_files {
            let repo_entry = repo_entry.unwrap();
            if !repo_entry.path().is_file() {
                continue;
            }

            let repo_profile_filename = repo_entry.file_name();
            let repo_profile_filename = repo_profile_filename.to_string_lossy();

            let lookup_in_binary = DEFAULT_PROFILES
                .iter()
                .find(|(profile_filename, _)| *profile_filename == repo_profile_filename);

            match lookup_in_binary {
                None => {
                    panic!("profile '{repo_profile_filename}' is missing in binary. Please add it to 'src/config_file.rs' 'DEFAULT_PROFILES' constant");
                }
                Some((binary_profile_name, binary_content)) => {
                    let repo_content = std::fs::read_to_string(repo_entry.path())
                        .expect("failed to read profile content");
                    assert_eq!(binary_content, &repo_content, "default profile content should match but doesn't for entry {binary_profile_name}");
                }
            }
        }
    }

    #[test]
    fn all_default_profiles_should_be_valid() {
        // get fixture api_schema
        // you can generate this within shopware by executing
        // composer run framework:schema:dump
        let raw_schema_content =
            std::fs::read_to_string("./fixtures/entity-schema-2024-08-01.json")
                .expect("failed to read entity-schema fixture");
        let api_schema: Entity = serde_json::from_str(&raw_schema_content)
            .expect("failed to parse entity-schema fixture");

        // run through all included default profiles and verify them
        for (profile_filename, profile_content) in DEFAULT_PROFILES {
            let profile: Profile = serde_yaml::from_str(&profile_content).expect(&format!(
                "failed to parse default profile '{profile_filename}'"
            ));

            validate_paths_for_entity(&profile.entity, &profile.mappings, &api_schema).expect(
                &format!("failed to validate entity path's for default profile {profile_filename}"),
            );
        }
    }
}
