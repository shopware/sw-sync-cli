//! Everything needed for communicating with the Shopware API

pub mod filter;

use crate::api::filter::{Criteria, CriteriaFilter};
use crate::config_file::Credentials;
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{header, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct SwClient {
    client: Client,
    credentials: Arc<Credentials>,
    access_token: Arc<Mutex<String>>,
}

impl SwClient {
    pub fn new(credentials: Credentials) -> anyhow::Result<Self> {
        let mut default_headers = HeaderMap::default();
        // This header is needed, otherwise the response would be "application/vnd.api+json" (by default)
        // and that doesn't have the association data as part of the entity object
        default_headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        let client = Client::builder()
            // workaround for long-running requests,
            // see https://github.com/hyperium/hyper/issues/2312#issuecomment-1411360500
            .pool_max_idle_per_host(0)
            .timeout(Duration::from_secs(15))
            .default_headers(default_headers)
            .build()?;
        let credentials = Arc::new(credentials);
        let auth_response = Self::authenticate(&client, credentials.as_ref())?;

        println!("Shopware API client created and authenticated");
        Ok(Self {
            client,
            credentials,
            access_token: Arc::new(Mutex::new(auth_response.access_token)),
        })
    }

    pub fn get_languages(&self) -> Result<IsoLanguageList, SwApiError> {
        let mut page = 1;
        let mut language_list: HashMap<String, String> = HashMap::new();

        let total = self.get_total("language", &[])?;

        let access_token = self.access_token.lock().unwrap().clone();
        while language_list.len() < total as usize {
            let mut criteria = Criteria {
                page,
                limit: Some(Criteria::MAX_LIMIT),
                fields: vec!["id".to_string(), "locale.code".to_string()],
                ..Default::default()
            };

            criteria.add_association("locale");

            let response = {
                self.client
                    .post(format!("{}/api/search/language", self.credentials.base_url))
                    .bearer_auth(&access_token)
                    .json(&criteria)
                    .send()?
            };

            let value: LanguageLocaleSearchResponse = Self::deserialize(response)?;
            for item in value.data {
                language_list.insert(item.locale.code, item.id);
            }

            page += 1;
        }

        Ok(IsoLanguageList {
            data: language_list,
        })
    }

    pub fn sync<S: Into<String>, T: Serialize>(
        &self,
        entity: S,
        action: SyncAction,
        payload: &[T],
    ) -> Result<(), SwApiError> {
        let entity: String = entity.into();
        // ToDo: implement retry on auth fail
        let access_token = self.access_token.lock().unwrap().clone();
        let body = SyncBody {
            write_data: SyncOperation {
                entity: entity.clone(),
                action,
                payload,
            },
        };

        let response = {
            let start_instant = Instant::now();
            println!(
                "sync {:?} '{}' with payload size {}",
                action,
                &entity,
                payload.len()
            );
            let res = self
                .client
                .post(format!("{}/api/_action/sync", self.credentials.base_url))
                .bearer_auth(access_token)
                .header("single-operation", 1)
                .header("indexing-behavior", "disable-indexing")
                .header("sw-skip-trigger-flow", 1)
                .json(&body)
                .send()?;
            println!(
                "sync request finished after {} ms",
                start_instant.elapsed().as_millis()
            );
            res
        };

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response)?;
            return Err(SwApiError::Server(status, body));
        }

        Ok(())
    }

    pub fn entity_schema(&self) -> Result<Entity, SwApiError> {
        // ToDo: implement retry on auth fail
        let access_token = self.access_token.lock().unwrap().clone();
        let response = {
            self.client
                .get(format!(
                    "{}/api/_info/entity-schema.json",
                    self.credentials.base_url
                ))
                .bearer_auth(access_token)
                .send()?
        };

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response)?;
            return Err(SwApiError::Server(status, body));
        }

        let value = Self::deserialize(response)?;
        Ok(value)
    }

    pub fn get_total(&self, entity: &str, filter: &[CriteriaFilter]) -> Result<u64, SwApiError> {
        // entity needs to be provided as kebab-case instead of snake_case
        let entity = entity.replace('_', "-");

        // ToDo: implement retry on auth fail
        let access_token = self.access_token.lock().unwrap().clone();

        let response = {
            self.client
                .post(format!(
                    "{}/api/search/{}",
                    self.credentials.base_url, entity
                ))
                .bearer_auth(access_token)
                .json(&json!({
                    "limit": 1,
                    "filter": filter,
                    "aggregations": [
                        {
                          "name": "count",
                          "type": "count",
                          "field": "id"
                        }
                    ]
                }))
                .send()?
        };

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response)?;
            return Err(SwApiError::Server(status, body));
        }

        let value: serde_json::Value = Self::deserialize(response)?;

        let count = value
            .pointer("/aggregations/count/count")
            .expect("failed to get /aggregations/count/count from response");
        let count = count
            .as_u64()
            .expect("count aggregation value is not a unsigned integer");

        Ok(count)
    }

    pub fn list(&self, entity: &str, criteria: &Criteria) -> Result<SwListResponse, SwApiError> {
        // entity needs to be provided as kebab-case instead of snake_case
        let entity = entity.replace('_', "-");

        // ToDo: implement retry on auth fail
        let access_token = self.access_token.lock().unwrap().clone();
        let response = {
            let start_instant = Instant::now();

            if let Some(limit) = criteria.limit {
                println!(
                    "fetching page {} of '{}' with limit {}",
                    criteria.page, entity, limit
                );
            } else {
                println!("fetching page {} of '{}'", criteria.page, entity);
            }

            let res = self
                .client
                .post(format!(
                    "{}/api/search/{}",
                    self.credentials.base_url, entity
                ))
                .bearer_auth(access_token)
                .json(criteria)
                .send()?;
            println!(
                "search request finished after {} ms",
                start_instant.elapsed().as_millis()
            );
            res
        };

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response)?;
            return Err(SwApiError::Server(status, body));
        }

        let value: SwListResponse = Self::deserialize(response)?;

        Ok(value)
    }

    fn authenticate(
        client: &Client,
        credentials: &Credentials,
    ) -> Result<AuthResponse, SwApiError> {
        let response = client
            .post(format!("{}/api/oauth/token", credentials.base_url))
            .json(&AuthBody {
                grant_type: "client_credentials".into(),
                client_id: credentials.access_key_id.clone(),
                client_secret: credentials.access_key_secret.clone(),
            })
            .send()?;

        if !response.status().is_success() {
            let status = response.status();
            let body: serde_json::Value = Self::deserialize(response)?;
            return Err(SwApiError::AuthFailed(
                status,
                serde_json::to_string_pretty(&body)?,
            ));
        }

        let res = Self::deserialize(response)?;

        Ok(res)
    }

    pub fn index(&self, skip: Vec<String>) -> Result<(), SwApiError> {
        let access_token = self.access_token.lock().unwrap().clone();

        let response = self
            .client
            .post(format!("{}/api/_action/index", self.credentials.base_url))
            .bearer_auth(access_token)
            .json(&IndexBody { skip })
            .send()?;

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response)?;
            return Err(SwApiError::Server(status, body));
        }

        Ok(())
    }

    fn deserialize<T>(response: Response) -> Result<T, SwApiError>
    where
        T: for<'a> Deserialize<'a> + Debug + Send + 'static,
    {
        let bytes = response.bytes()?;

        // expensive for large json objects
        let result = match serde_json::from_slice(&bytes) {
            Ok(t) => Ok(t),
            Err(_e) => {
                // try to parse any json
                match serde_json::from_slice::<serde_json::Value>(&bytes) {
                    Ok(json_value) => Err(SwApiError::DeserializeIntoSchema(
                        std::any::type_name::<T>().to_string(),
                        serde_json::to_string_pretty(&json_value)
                            .expect("json pretty printing shouldn't fail"),
                    )),
                    Err(_e) => Err(SwApiError::DeserializeIntoSchema(
                        std::any::type_name::<T>().to_string(),
                        String::from_utf8_lossy(&bytes).into_owned(),
                    )),
                }
            }
        };
        result
    }
}

#[derive(Debug, Serialize)]
struct IndexBody {
    skip: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AuthBody {
    grant_type: String,
    client_id: String,
    client_secret: String,
}

#[derive(Debug, Deserialize)]
struct AuthResponse {
    // token_type: String,
    // expires_in: u32,
    access_token: String,
}

#[derive(Debug, Serialize)]
struct SyncBody<'a, T> {
    write_data: SyncOperation<'a, T>,
}

#[derive(Debug, Serialize)]
struct SyncOperation<'a, T> {
    entity: String,
    action: SyncAction,
    payload: &'a [T],
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncAction {
    Upsert,
    Delete,
}

#[derive(Debug, Error)]
pub enum SwApiError {
    #[error("The server returned an {0} error response:\n{1:#?}")]
    Server(StatusCode, SwErrorBody),
    #[error("Request error: {0}")]
    Request(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to deserialize json into schema of type {0}, got:\n{1}")]
    DeserializeIntoSchema(String, String),
    #[error("Failed to authenticate, got {0} with body:\n{1}")]
    AuthFailed(StatusCode, String),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SwErrorBody {
    pub errors: Vec<SwError>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SwError {
    pub code: String,
    pub detail: String,
    pub source: SwErrorSource,
    pub template: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SwErrorSource {
    pub pointer: String,
}

#[derive(Debug, Deserialize)]
pub struct SwListResponse {
    pub data: Vec<Entity>,
}

#[derive(Debug, Deserialize)]
pub struct Language {
    pub id: String,
    pub locale: Locale,
}

#[derive(Debug, Deserialize)]
pub struct Locale {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct LanguageLocaleSearchResponse {
    pub data: Vec<Language>,
}

#[derive(Debug, Clone, Default)]
pub struct IsoLanguageList {
    pub data: HashMap<String, String>,
}

impl IsoLanguageList {
    pub fn get_language_id_by_iso_code(&self, iso_code: &str) -> String {
        match self.data.get(iso_code) {
            Some(id) => id.to_string(),
            None => {
                println!("Language with iso code '{}' not found", iso_code);
                "".to_string()
            }
        }
    }
}

pub type Entity = serde_json::Map<String, serde_json::Value>;

#[cfg(test)]
mod tests {
    use crate::api::IsoLanguageList;
    use std::collections::HashMap;

    #[test]
    fn test_iso_language_list() {
        let mut language_list_inner: HashMap<String, String> = HashMap::new();
        language_list_inner.insert(
            "de-DE".to_string(),
            "cf8eb267dd2a4c54be07bf4b50d65ab5".to_string(),
        );
        language_list_inner.insert(
            "en-GB".to_string(),
            "a13966f91ef24dcabccf1668e3618955".to_string(),
        );

        let locale_list = IsoLanguageList {
            data: language_list_inner,
        };

        assert_eq!(
            locale_list.get_language_id_by_iso_code("de-DE"),
            "cf8eb267dd2a4c54be07bf4b50d65ab5"
        );
        assert_eq!(
            locale_list.get_language_id_by_iso_code("en-GB"),
            "a13966f91ef24dcabccf1668e3618955"
        );
        assert_eq!(locale_list.get_language_id_by_iso_code("en-US"), "");
    }
}
