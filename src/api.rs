use crate::config::Credentials;
use anyhow::anyhow;
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
pub struct SwClient {
    client: Client,
    /// Limits the number of "in-flight" requests
    in_flight_semaphore: Arc<Semaphore>,
    credentials: Arc<Credentials>,
    access_token: Arc<Mutex<String>>,
}

impl SwClient {
    pub async fn new(credentials: Credentials, in_flight_limit: usize) -> anyhow::Result<Self> {
        let client = Client::builder().timeout(Duration::from_secs(10)).build()?;
        let credentials = Arc::new(credentials);
        let auth_response = Self::authenticate(&client, credentials.as_ref()).await?;

        Ok(Self {
            client,
            in_flight_semaphore: Arc::new(Semaphore::new(in_flight_limit)),
            credentials,
            access_token: Arc::new(Mutex::new(auth_response.access_token)),
        })
    }

    pub async fn sync<S: Into<String>, T: Serialize>(
        &self,
        entity: S,
        action: SyncAction,
        payload: &[T],
    ) -> Result<(), SwApiError> {
        let entity: String = entity.into();
        let start_instant = Instant::now();
        println!(
            "sync {:?} '{}' with payload size {}",
            action,
            &entity,
            payload.len()
        );
        // ToDo: implement retry on auth fail
        let access_token = self.access_token.lock().unwrap().clone();
        let body = SyncBody {
            write_data: SyncOperation {
                entity,
                action,
                payload,
            },
        };

        let response = {
            let _lock = self.in_flight_semaphore.acquire();
            self.client
                .post(format!("{}/api/_action/sync", self.credentials.base_url))
                .bearer_auth(access_token)
                .header("single-operation", 1)
                .header("indexing-behavior", "disable-indexing")
                .header("sw-skip-trigger-flow", 1)
                .json(&body)
                .send()
                .await?
        };

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response).await?;
            return Err(SwApiError::Server(status, body));
        }

        println!(
            "sync finished after {} ms",
            start_instant.elapsed().as_millis()
        );

        Ok(())
    }

    pub async fn entity_schema(
        &self,
    ) -> Result<serde_json::Map<String, serde_json::Value>, SwApiError> {
        // ToDo: implement retry on auth fail
        let access_token = self.access_token.lock().unwrap().clone();
        let response = {
            let _lock = self.in_flight_semaphore.acquire();
            self.client
                .get(format!(
                    "{}/api/_info/entity-schema.json",
                    self.credentials.base_url
                ))
                .bearer_auth(access_token)
                .send()
                .await?
        };

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response).await?;
            return Err(SwApiError::Server(status, body));
        }

        let value = Self::deserialize(response).await?;
        Ok(value)
    }

    pub async fn get_total(
        &self,
        entity: &str,
        filter: &[CriteriaFilter],
    ) -> Result<u64, SwApiError> {
        // entity needs to be provided as kebab-case instead of snake_case
        let entity = entity.replace('_', "-");

        // ToDo: implement retry on auth fail
        let access_token = self.access_token.lock().unwrap().clone();

        let response = {
            let _lock = self.in_flight_semaphore.acquire();
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
                .send()
                .await?
        };

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response).await?;
            return Err(SwApiError::Server(status, body));
        }

        let value: serde_json::Value = Self::deserialize(response).await?;

        let count = value
            .pointer("/aggregations/count/count")
            .expect("failed to get /aggregations/count/count from response");
        let count = count
            .as_u64()
            .expect("count aggregation value is not a unsigned integer");

        Ok(count)
    }

    pub async fn list(
        &self,
        entity: &str,
        criteria: &Criteria,
    ) -> Result<SwListResponse, SwApiError> {
        let start_instant = Instant::now();
        // entity needs to be provided as kebab-case instead of snake_case
        let entity = entity.replace('_', "-");

        // ToDo: implement retry on auth fail
        let access_token = self.access_token.lock().unwrap().clone();
        let response = {
            let _lock = self.in_flight_semaphore.acquire();
            self.client
                .post(format!(
                    "{}/api/search/{}",
                    self.credentials.base_url, entity
                ))
                .bearer_auth(access_token)
                .json(criteria)
                .send()
                .await?
        };

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response).await?;
            return Err(SwApiError::Server(status, body));
        }

        let value: SwListResponse = Self::deserialize(response).await?;

        println!(
            "search request finished after {} ms",
            start_instant.elapsed().as_millis()
        );

        Ok(value)
    }

    async fn authenticate(
        client: &Client,
        credentials: &Credentials,
    ) -> anyhow::Result<AuthResponse> {
        let response = client
            .post(format!("{}/api/oauth/token", credentials.base_url))
            .json(&AuthBody {
                grant_type: "client_credentials".into(),
                client_id: credentials.access_key_id.clone(),
                client_secret: credentials.access_key_secret.clone(),
            })
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body: serde_json::Value = response.json().await?;
            return Err(anyhow!(
                "Failed to authenticate, got {} with body:\n{}",
                status,
                serde_json::to_string_pretty(&body)?
            ));
        }

        let res = Self::deserialize(response).await?;

        Ok(res)
    }

    async fn deserialize<T: for<'a> Deserialize<'a>>(response: Response) -> Result<T, SwApiError> {
        let text = response.text().await?;

        match serde_json::from_str(&text) {
            Ok(t) => Ok(t),
            Err(_e) => {
                let body: serde_json::Value = serde_json::from_str(&text)?;
                Err(SwApiError::DeserializeIntoSchema(
                    serde_json::to_string_pretty(&body)?,
                ))
            }
        }
    }
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
    #[error("failed to deserialize json into schema:\n{0}")]
    DeserializeIntoSchema(String),
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
    pub data: Vec<SwListEntity>,
}

#[derive(Debug, Deserialize)]
pub struct SwListEntity {
    pub id: String,
    pub r#type: String,
    pub attributes: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct Criteria {
    pub limit: u64,
    pub page: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<CriteriaFilter>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<CriteriaSorting>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub associations: BTreeMap<String, EmptyObject>,
}

impl Default for Criteria {
    fn default() -> Self {
        Self {
            limit: Self::MAX_LIMIT,
            page: 1,
            sort: vec![],
            filter: vec![],
            associations: BTreeMap::new(),
        }
    }
}

impl Criteria {
    /// Maximum limit accepted by the API server
    pub const MAX_LIMIT: u64 = 500;

    pub fn add_filter(&mut self, filter: CriteriaFilter) {
        self.filter.push(filter);
    }

    pub fn add_sorting(&mut self, sorting: CriteriaSorting) {
        self.sort.push(sorting);
    }

    pub fn add_association<S: Into<String>>(&mut self, association: S) {
        self.associations.insert(association.into(), EmptyObject {});
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CriteriaSorting {
    pub field: String,
    pub order: CriteriaSortingOrder,
}

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
pub enum CriteriaSortingOrder {
    #[serde(rename = "ASC")]
    Ascending,
    #[serde(rename = "DESC")]
    Descending,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CriteriaFilter {
    Equals {
        field: String,
        value: serde_json::Value,
    },
}

#[derive(Debug, Serialize)]
pub struct EmptyObject {
    // no fields
}

#[cfg(test)]
mod tests {
    use crate::api::{Criteria, CriteriaFilter, CriteriaSorting, CriteriaSortingOrder};

    #[test]
    fn criteria_serialize_association() {
        let mut criteria = Criteria {
            limit: 10,
            page: 2,
            ..Default::default()
        };
        criteria.add_association("manufacturer");
        criteria.add_association("cover");

        let json = serde_json::to_string(&criteria).unwrap();
        assert_eq!(
            json,
            "{\"limit\":10,\"page\":2,\"associations\":{\"cover\":{},\"manufacturer\":{}}}"
        );
    }

    #[test]
    fn criteria_serialize_sorting() {
        let mut criteria = Criteria {
            limit: 10,
            page: 2,
            ..Default::default()
        };
        criteria.add_sorting(CriteriaSorting {
            field: "manufacturerId".to_string(),
            order: CriteriaSortingOrder::Descending,
        });

        let json = serde_json::to_string(&criteria).unwrap();
        assert_eq!(json, "{\"limit\":10,\"page\":2,\"sort\":[{\"field\":\"manufacturerId\",\"order\":\"DESC\"}]}");
    }

    #[test]
    fn criteria_serialize_filter() {
        let mut criteria = Criteria {
            limit: 10,
            page: 2,
            ..Default::default()
        };
        criteria.add_filter(CriteriaFilter::Equals {
            field: "parentId".to_string(),
            value: serde_json::Value::Null,
        });

        let json = serde_json::to_string(&criteria).unwrap();
        assert_eq!(json, "{\"limit\":10,\"page\":2,\"filter\":[{\"type\":\"Equals\",\"field\":\"parentId\",\"value\":null}]}");
    }
}
