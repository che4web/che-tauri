use std::{
    path::Path,
    sync::{Arc, RwLock},
};

use che_orm::Database;

use crate::{
    config::{AppConfig, RemoteConfig},
    error::ApiResult,
};

#[derive(Clone)]
pub struct AppState {
    config: Arc<RwLock<AppConfig>>,
    db: Database,
    auth_token: Arc<RwLock<Option<String>>>,
}

impl AppState {
    pub async fn from_config_file(path: impl AsRef<Path>) -> ApiResult<Self> {
        let config = AppConfig::from_file(path)?;
        let db = connect_database(&config.database.url).await?;

        Ok(Self::new(config, db))
    }

    pub fn new(config: AppConfig, db: Database) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            db,
            auth_token: Arc::new(RwLock::new(None)),
        }
    }

    pub fn config(&self) -> AppConfig {
        self.config
            .read()
            .expect("app config lock poisoned")
            .clone()
    }

    pub fn remote_config(&self) -> Option<RemoteConfig> {
        self.config().remote
    }

    pub fn set_remote_config(&self, remote: Option<RemoteConfig>) {
        self.config
            .write()
            .expect("app config lock poisoned")
            .remote = remote;
    }

    pub fn db(&self) -> &Database {
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

impl std::fmt::Debug for AppState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AppState").finish_non_exhaustive()
    }
}

pub(crate) async fn connect_database(url: &str) -> Result<Database, che_orm::OrmError> {
    let path = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    // rusqlite enables SQLite URI handling for `file:` paths. Keep query
    // parameters intact so SQLite enforces mode=ro/rw/rwc and cache settings.
    let uri = if path.starts_with("file:") || !path.contains('?') {
        path.to_string()
    } else {
        format!("file:{path}")
    };
    let (filename, query) = uri.split_once('?').unwrap_or((&uri, ""));
    let filename = filename.strip_prefix("file:").unwrap_or(filename);
    let memory =
        filename == ":memory:" || query.split('&').any(|parameter| parameter == "mode=memory");
    // Every in-memory database must use one pooled connection, including URIs.
    Database::connect_with_pool_size(uri, if memory { 1 } else { 4 }).await
}
