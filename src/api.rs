use crate::config::Credentials;
use anyhow::anyhow;
use anyhow::Context;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
        payload: Vec<T>,
    ) -> anyhow::Result<()> {
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
            let body: serde_json::Value = res.json().await?;
            return Err(anyhow!(
                "Sync request failed, status {} and body:\n{:#?}",
                status,
                body
            ));
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

        let res = Self::deserialize(res).await?;

        Ok(res)
    }

    async fn deserialize<T: for<'a> Deserialize<'a>>(response: Response) -> anyhow::Result<T> {
        let text = response.text().await?;

        match serde_json::from_str(&text) {
            Ok(t) => Ok(t),
            Err(e) => {
                let body: serde_json::Value = serde_json::from_str(&text)?;

                Err(e).context(format!(
                    "failed to deserialize json into schema:\n{:#?}",
                    body
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
    token_type: String,
    expires_in: u32,
    access_token: String,
}

#[derive(Debug, Serialize)]
struct SyncBody<T> {
    write_data: SyncOperation<T>,
}

#[derive(Debug, Serialize)]
struct SyncOperation<T> {
    entity: String,
    action: SyncAction,
    payload: Vec<T>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncAction {
    Upsert,
    Delete,
}
