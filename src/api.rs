use crate::config::Credentials;
use anyhow::anyhow;
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
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
    pub async fn new(credentials: Credentials) -> anyhow::Result<Self> {
        let client = Client::builder().timeout(Duration::from_secs(10)).build()?;
        let credentials = Arc::new(credentials);
        let auth_response = Self::authenticate(&client, credentials.as_ref()).await?;

        Ok(Self {
            client,
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
            "sync {:?} {} with payload size {}",
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

        if !res.status().is_success() {
            let status = res.status();
            let body: SwErrorBody = Self::deserialize(res).await?;
            return Err(SwApiError::Server(status, body));
        }

        println!(
            "sync finished after {} ms",
            start_instant.elapsed().as_millis()
        );

        Ok(())
    }

    async fn authenticate(
        client: &Client,
        credentials: &Credentials,
    ) -> anyhow::Result<AuthResponse> {
        let res = client
            .post(format!("{}/api/oauth/token", credentials.base_url))
            .json(&AuthBody {
                grant_type: "client_credentials".into(),
                client_id: credentials.access_key_id.clone(),
                client_secret: credentials.access_key_secret.clone(),
            })
            .send()
            .await?;

        if !res.status().is_success() {
            let status = res.status();
            let body: serde_json::Value = res.json().await?;
            return Err(anyhow!(
                "Failed to authenticate, got {} with body:\n{}",
                status,
                serde_json::to_string_pretty(&body)?
            ));
        }

        let res = Self::deserialize(res).await?;

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
