//! Everything needed for communicating with the Shopware API

pub mod filter;

use crate::api::filter::{Criteria, CriteriaFilter};
use crate::config_file::Credentials;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{header, Client, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::{Mutex, Semaphore};

#[derive(Debug, Clone)]
pub struct SwClient {
    client: Client,
    /// Limits the number of "in-flight" requests
    in_flight_semaphore: Arc<Semaphore>,
    credentials: Arc<Credentials>,
    access_token: Arc<Mutex<String>>,
}

impl SwClient {
    pub const DEFAULT_IN_FLIGHT: usize = 10;

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
            "Shopware API client with in_flight_limit={in_flight_limit} created and authenticated"
        );
        Ok(Self {
            client,
            in_flight_semaphore: Arc::new(Semaphore::new(in_flight_limit)),
            credentials,
            access_token: Arc::new(Mutex::new(auth_response.access_token)),
        })
    }

    pub async fn sync<S: Into<String>, T: Serialize + Debug>(
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

        let mut headers = HeaderMap::new();
        headers.insert("single-operation", HeaderValue::from_static("1"));
        headers.insert(
            "indexing-behavior",
            HeaderValue::from_static("disable-indexing"),
        );
        headers.insert("sw-skip-trigger-flow", HeaderValue::from_static("1"));

        let (response, duration) = self
            .handle_authenticated_request(
                Method::POST,
                "/api/_action/sync",
                Some(&body),
                Some(headers),
                true,
            )
            .await?;

        println!(
            "sync request finished after {} ms",
            duration.unwrap().as_millis()
        );

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response).await?;
            return Err(SwApiError::Server(status, body));
        }

        Ok(())
    }

    pub async fn entity_schema(&self) -> Result<Entity, SwApiError> {
        let (response, _) = self
            .handle_authenticated_request::<()>(
                Method::GET,
                "/api/_info/entity-schema.json",
                None,
                None,
                false,
            )
            .await?;

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

        let (response, _) = self
            .handle_authenticated_request(
                Method::POST,
                &format!("/api/search/{}", entity),
                Some(&body),
                None,
                false,
            )
            .await?;

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

        if let Some(limit) = criteria.limit {
            println!(
                "fetching page {} of '{}' with limit {}",
                criteria.page, entity, limit
            );
        } else {
            println!("fetching page {} of '{}'", criteria.page, entity);
        }

        let (response, duration) = self
            .handle_authenticated_request(
                Method::POST,
                &format!("/api/search/{}", entity),
                Some(criteria),
                None,
                true,
            )
            .await?;

        println!(
            "search request finished after {} ms",
            duration.unwrap().as_millis()
        );

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
    ) -> Result<AuthResponse, SwApiError> {
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
            return Err(SwApiError::AuthFailed(
                status,
                serde_json::to_string_pretty(&body)?,
            ));
        }

        let res = Self::deserialize(response).await?;

        Ok(res)
    }

    pub async fn index(&self, skip: Vec<String>) -> Result<(), SwApiError> {
        let (response, _) = self
            .handle_authenticated_request(
                Method::POST,
                "/api/_action/index",
                Some(&IndexBody { skip }),
                None,
                false,
            )
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body: SwErrorBody = Self::deserialize(response).await?;
            return Err(SwApiError::Server(status, body));
        }

        Ok(())
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
            worker_tx.send(result).unwrap();
        });
        worker_rx.await.unwrap()
    }

    async fn handle_authenticated_request<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&T>,
        additional_headers: Option<HeaderMap>,
        measure_time: bool,
    ) -> Result<(Response, Option<Duration>), SwApiError> {
        let url = format!("{}{}", self.credentials.base_url, path);
        let mut retry_count = 0;
        const MAX_RETRIES: u8 = 1;

        let mut request_builder = self.client.request(method, &url);

        if let Some(headers) = additional_headers {
            request_builder = request_builder.headers(headers);
        }

        if let Some(body_value) = body {
            request_builder = request_builder.json(body_value);
        }

        loop {
            let access_token = self.access_token.lock().await.clone();
            let request = request_builder
                .try_clone()
                .unwrap()
                .bearer_auth(&access_token);

            let _lock = self.in_flight_semaphore.acquire().await.unwrap();

            let start_time = if measure_time {
                Some(Instant::now())
            } else {
                None
            };

            let response = request.send().await?;

            if response.status() == StatusCode::UNAUTHORIZED && retry_count < MAX_RETRIES {
                // lock the access token
                let mut access_token_guard = self.access_token.lock().await;
                // compare the access token with the one we used to make the request
                if *access_token_guard != access_token {
                    // Another thread has already re-authenticated
                    continue;
                }

                // Perform re-authentication
                let auth_response = Self::authenticate(&self.client, &self.credentials).await?;
                let new_token = auth_response.access_token;
                *access_token_guard = new_token;

                retry_count += 1;
                continue;
            }

            let duration = start_time.map(|start_time| start_time.elapsed());

            return Ok((response, duration));
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
