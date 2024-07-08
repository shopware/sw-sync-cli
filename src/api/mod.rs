//! Everything needed for communicating with the Shopware API

pub mod filter;

use crate::api::filter::{Criteria, CriteriaFilter};
use crate::config_file::Credentials;
use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{header, Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Debug;
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
        let auth_response = Self::authenticate(&client, credentials.as_ref()).await?;

        println!(
            "Shopware API client with in_flight_limit={} created and authenticated",
            in_flight_limit
        );
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
            let _lock = self.in_flight_semaphore.acquire().await.unwrap();
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
                .send()
                .await?;
            println!(
                "sync request finished after {} ms",
                start_instant.elapsed().as_millis()
            );
            res
        };

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response).await?;
            return Err(SwApiError::Server(status, body));
        }

        Ok(())
    }

    pub async fn entity_schema(&self) -> Result<Entity, SwApiError> {
        // ToDo: implement retry on auth fail
        let access_token = self.access_token.lock().unwrap().clone();
        let response = {
            let _lock = self.in_flight_semaphore.acquire().await.unwrap();
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
            let _lock = self.in_flight_semaphore.acquire().await.unwrap();
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
        // entity needs to be provided as kebab-case instead of snake_case
        let entity = entity.replace('_', "-");

        // ToDo: implement retry on auth fail
        let access_token = self.access_token.lock().unwrap().clone();
        let response = {
            let _lock = self.in_flight_semaphore.acquire().await.unwrap();
            let start_instant = Instant::now();
            println!(
                "fetching page {} of '{}' with limit {}",
                criteria.page, entity, criteria.limit
            );
            let res = self
                .client
                .post(format!(
                    "{}/api/search/{}",
                    self.credentials.base_url, entity
                ))
                .bearer_auth(access_token)
                .json(criteria)
                .send()
                .await?;
            println!(
                "search request finished after {} ms",
                start_instant.elapsed().as_millis()
            );
            res
        };

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response).await?;
            return Err(SwApiError::Server(status, body));
        }

        let value: SwListResponse = Self::deserialize(response).await?;

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
            let body: serde_json::Value = Self::deserialize(response).await?;
            return Err(anyhow!(
                "Failed to authenticate, got {} with body:\n{}",
                status,
                serde_json::to_string_pretty(&body)?
            ));
        }

        let res = Self::deserialize(response).await?;

        Ok(res)
    }

    async fn deserialize<T>(response: Response) -> Result<T, SwApiError>
    where
        T: for<'a> Deserialize<'a> + Debug + Send + 'static,
    {
        let bytes = response.bytes().await?;

        // offload heavy deserialization (shopware json responses can get big) to worker thread
        // to not block this thread for too long doing async work
        let (worker_tx, worker_rx) = tokio::sync::oneshot::channel::<Result<T, SwApiError>>();
        rayon::spawn(move || {
            // expensive for lage json objects
            let result = match serde_json::from_slice(&bytes) {
                Ok(t) => Ok(t),
                Err(_e) => {
                    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    Err(SwApiError::DeserializeIntoSchema(
                        serde_json::to_string_pretty(&body).unwrap(),
                    ))
                }
            };
            worker_tx.send(result).unwrap();
        });
        worker_rx.await.unwrap()
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
    pub data: Vec<Entity>,
}

pub type Entity = serde_json::Map<String, serde_json::Value>;
