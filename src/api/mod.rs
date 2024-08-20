//! Everything needed for communicating with the Shopware API

pub mod filter;

use crate::api::filter::{Criteria, CriteriaFilter};
use crate::config_file::Credentials;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{header, Method, StatusCode};
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

        while language_list.len() < total as usize {
            let mut criteria = Criteria {
                page,
                limit: Some(Criteria::MAX_LIMIT),
                fields: vec!["id".to_string(), "locale.code".to_string()],
                ..Default::default()
            };

            criteria.add_association("locale");

            let list: SwListResponse<Language> = self.list("language", &criteria)?;
            for item in list.data {
                language_list.insert(item.locale.code, item.id);
            }

            page += 1;
        }

        Ok(IsoLanguageList {
            data: language_list,
        })
    }

    pub fn get_currencies(&self) -> Result<CurrencyList, SwApiError> {
        let mut page = 1;
        let mut currency_list: HashMap<String, String> = HashMap::new();

        let total = self.get_total("currency", &[])?;

        while currency_list.len() < total as usize {
            let criteria = Criteria {
                page,
                limit: Some(Criteria::MAX_LIMIT),
                fields: vec!["id".to_string(), "isoCode".to_string()],
                ..Default::default()
            };

            let list: SwListResponse<Currency> = self.list("currency", &criteria)?;
            for item in list.data {
                currency_list.insert(item.iso_code, item.id);
            }

            page += 1;
        }

        Ok(CurrencyList {
            data: currency_list,
        })
    }

    pub fn sync<S: Into<String>, T: Serialize + Debug>(
        &self,
        entity: S,
        action: SyncAction,
        payload: &[T],
    ) -> Result<(), SwApiError> {
        let entity: String = entity.into();
        let body = SyncBody {
            write_data: SyncOperation {
                entity: entity.clone(),
                action,
                payload,
            },
        };

        println!(
            "sync {:?} '{}' with payload size {}",
            action,
            &entity,
            payload.len()
        );

        let request_builder = self
            .client
            .request(
                Method::POST,
                format!("{}/api/_action/sync", self.credentials.base_url),
            )
            .header("single-operation", "1")
            .header("indexing-behavior", "disable-indexing")
            .header("sw-skip-trigger-flow", "1")
            .json(&body);

        let response = self.handle_authenticated_request(request_builder)?;

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response)?;
            return Err(SwApiError::Server(status, body));
        }

        Ok(())
    }

    pub fn entity_schema(&self) -> Result<Entity, SwApiError> {
        let request_builder = self.client.request(
            Method::GET,
            format!("{}/api/_info/entity-schema.json", self.credentials.base_url),
        );

        let response = self.handle_authenticated_request(request_builder)?;

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
        let body = json!({
            "limit": 1,
            "filter": filter,
            "aggregations": [
                {
                  "name": "count",
                  "type": "count",
                  "field": "id"
                }
            ]
        });

        let request_builder = self
            .client
            .request(
                Method::POST,
                format!("{}/api/search/{}", self.credentials.base_url, entity),
            )
            .json(&body);

        let response = self.handle_authenticated_request(request_builder)?;

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

    pub fn list<T>(
        &self,
        entity: &str,
        criteria: &Criteria,
    ) -> Result<SwListResponse<T>, SwApiError>
    where
        T: for<'a> Deserialize<'a> + Debug + Send + 'static,
    {
        // entity needs to be provided as kebab-case instead of snake_case
        let entity = entity.replace('_', "-");

        if let Some(limit) = criteria.limit {
            println!(
                "fetching page {} of '{}' with limit {}",
                criteria.page, entity, limit
            );
        } else {
            println!("fetching page {} of '{}'", criteria.page, entity);
        }

        let request_builder = self
            .client
            .request(
                Method::POST,
                format!("{}/api/search/{}", self.credentials.base_url, entity),
            )
            .json(criteria);

        let response = self.handle_authenticated_request(request_builder)?;

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response)?;
            return Err(SwApiError::Server(status, body));
        }

        let value: SwListResponse<T> = Self::deserialize(response)?;

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
            let body: SwErrorBody = Self::deserialize(response)?;
            return Err(SwApiError::AuthFailed(
                status,
                serde_json::to_string_pretty(&body)?,
            ));
        }

        let res = Self::deserialize(response)?;

        Ok(res)
    }

    pub fn index(&self, skip: Vec<String>) -> Result<(), SwApiError> {
        let request_builder = self
            .client
            .request(
                Method::POST,
                format!("{}/api/_action/index", self.credentials.base_url),
            )
            .json(&IndexBody { skip });

        let response = self.handle_authenticated_request(request_builder)?;

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

    fn handle_authenticated_request(
        &self,
        request_builder: RequestBuilder,
    ) -> Result<Response, SwApiError> {
        let mut try_count = 0;
        const MAX_RETRIES: u8 = 1;
        let binding = request_builder.try_clone().unwrap().build().unwrap();
        let path = binding.url().path();

        loop {
            let access_token = self.access_token.lock().unwrap().clone();
            let request = request_builder
                .try_clone()
                .unwrap()
                .bearer_auth(&access_token);

            let start_time = Instant::now();
            let response = request.send()?;

            if response.status() == StatusCode::UNAUTHORIZED && try_count < MAX_RETRIES {
                // lock the access token
                let mut access_token_guard = self.access_token.lock().unwrap();
                // compare the access token with the one we used to make the request
                if *access_token_guard != access_token {
                    // Another thread has already re-authenticated
                    continue;
                }

                // Perform re-authentication
                let auth_response = Self::authenticate(&self.client, &self.credentials)?;
                let new_token = auth_response.access_token;
                *access_token_guard = new_token;

                try_count += 1;
                continue;
            }

            let duration = start_time.elapsed();
            println!(
                "{} request finished after {} ms",
                path,
                duration.as_millis()
            );

            return Ok(response);
        }
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

impl SwErrorBody {
    pub fn check_for_error_code(&self, error_code: &str) -> bool {
        self.errors.iter().any(|error| match error {
            SwError::GenericError { code, .. } if code.eq(error_code) => true,
            SwError::WriteError { code, .. } if code.eq(error_code) => true,
            _ => false,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SwError {
    WriteError {
        code: String,
        detail: String,
        source: SwErrorSource,
        template: String,
    },
    GenericError {
        code: String,
        detail: Option<String>,
        status: String,
        title: String,
    },
}

impl SwError {
    pub const ERROR_CODE_DEADLOCK: &'static str = "1213";
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SwErrorSource {
    pub pointer: String,
}

#[derive(Debug, Deserialize)]
pub struct SwListResponse<T> {
    pub data: Vec<T>,
}

#[derive(Debug, Deserialize)]
pub struct Currency {
    pub id: String,
    #[serde(rename = "isoCode")]
    pub iso_code: String,
}

#[derive(Debug, Clone, Default)]
pub struct CurrencyList {
    pub data: HashMap<String, String>,
}

impl CurrencyList {
    pub fn get_currency_id_by_iso_code(&self, iso_code: &str) -> String {
        match self.data.get(iso_code) {
            Some(id) => id.to_string(),
            None => {
                println!("Currency with iso code '{}' not found", iso_code);
                "".to_string()
            }
        }
    }
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
    use crate::api::CurrencyList;
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

    #[test]
    fn test_currency_list() {
        let mut currency_list_inner: HashMap<String, String> = HashMap::new();
        currency_list_inner.insert(
            "EUR".to_string(),
            "a55d590baf2c432999f650f421f25eb6".to_string(),
        );
        currency_list_inner.insert(
            "USD".to_string(),
            "cae49554610b4df2be0fbd61be51f66d".to_string(),
        );

        let currency_list = CurrencyList {
            data: currency_list_inner,
        };

        assert_eq!(
            currency_list.get_currency_id_by_iso_code("EUR"),
            "a55d590baf2c432999f650f421f25eb6"
        );
        assert_eq!(
            currency_list.get_currency_id_by_iso_code("USD"),
            "cae49554610b4df2be0fbd61be51f66d"
        );
        assert_eq!(currency_list.get_currency_id_by_iso_code("GBP"), "");
    }
}
