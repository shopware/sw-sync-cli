use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Credentials {
    pub base_url: String,
    pub access_key_id: String,
    pub access_key_secret: String,
}
