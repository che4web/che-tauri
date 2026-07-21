use std::path::Path;

use crate::error::ApiResult;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AppConfig {
    pub database: DatabaseConfig,
    pub remote: Option<RemoteConfig>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RemoteConfig {
    pub base_url: String,
}

impl AppConfig {
    pub fn from_file(path: impl AsRef<Path>) -> ApiResult<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }
}
