use std::{
    path::Path,
    sync::{Arc, RwLock},
};

use che_orm::SqliteBackend;

use crate::{config::{AppConfig, RemoteConfig}, error::ApiResult};

#[derive(Debug, Clone)]
pub struct AppState {
    config: Arc<RwLock<AppConfig>>,
    db: SqliteBackend,
    auth_token: Arc<RwLock<Option<String>>>,
}

impl AppState {
    pub async fn from_config_file(path: impl AsRef<Path>) -> ApiResult<Self> {
        let config = AppConfig::from_file(path)?;
        let db = SqliteBackend::connect(&config.database.url).await?;

        Ok(Self::new(config, db))
    }

    pub fn new(config: AppConfig, db: SqliteBackend) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            db,
            auth_token: Arc::new(RwLock::new(None)),
        }
    }

    pub fn config(&self) -> AppConfig {
        self.config.read().expect("app config lock poisoned").clone()
    }

    pub fn remote_config(&self) -> Option<RemoteConfig> {
        self.config().remote
    }

    pub fn set_remote_config(&self, remote: Option<RemoteConfig>) {
        self.config.write().expect("app config lock poisoned").remote = remote;
    }

    pub fn db(&self) -> &SqliteBackend {
        &self.db
    }

    pub fn set_auth_token(&self, token: impl Into<String>) {
        *self.auth_token.write().expect("auth token lock poisoned") = Some(token.into());
    }

    pub fn clear_auth_token(&self) {
        *self.auth_token.write().expect("auth token lock poisoned") = None;
    }

    pub fn auth_token(&self) -> Option<String> {
        self.auth_token
            .read()
            .expect("auth token lock poisoned")
            .clone()
    }
}
