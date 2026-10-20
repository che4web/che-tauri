use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, RwLock},
};

use che_orm::Database;

use crate::{
    config::{AppConfig, RemoteConfig},
    error::{ApiError, ApiResult},
};

#[derive(Clone)]
pub struct AppState {
    config: Arc<RwLock<AppConfig>>,
    db: Database,
    auth: Arc<RwLock<AuthState>>,
    cache: Arc<tokio::sync::Mutex<CacheState>>,
    cache_tables: Arc<RwLock<HashSet<&'static str>>>,
}

#[derive(Default)]
struct AuthState {
    token: Option<String>,
    generation: u64,
}

#[derive(Default)]
struct CacheState {
    generation: Option<u64>,
}

pub(crate) struct CacheSession<'a> {
    _guard: tokio::sync::MutexGuard<'a, CacheState>,
    generation: u64,
}

impl CacheSession<'_> {
    pub(crate) fn finish<T>(&self, state: &AppState, result: ApiResult<T>) -> ApiResult<T> {
        if state.auth.read().expect("auth lock poisoned").generation != self.generation {
            return Err(ApiError::new(
                "not_authenticated",
                "remote session changed during request",
            ));
        }
        result
    }
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
            auth: Arc::new(RwLock::new(AuthState::default())),
            cache: Arc::new(tokio::sync::Mutex::new(CacheState::default())),
            cache_tables: Arc::new(RwLock::new(HashSet::new())),
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
        let mut config = self.config.write().expect("app config lock poisoned");
        if config.remote != remote {
            let mut auth = self.auth.write().expect("auth lock poisoned");
            auth.token = None;
            auth.generation += 1;
            config.remote = remote;
        }
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    pub fn set_auth_token(&self, token: impl Into<String>) {
        let token = token.into();
        let mut auth = self.auth.write().expect("auth lock poisoned");
        if auth.token.as_ref() != Some(&token) {
            auth.token = Some(token);
            auth.generation += 1;
        }
    }

    pub fn clear_auth_token(&self) {
        let mut auth = self.auth.write().expect("auth lock poisoned");
        auth.token = None;
        auth.generation += 1;
    }

    pub fn auth_token(&self) -> Option<String> {
        self.auth.read().expect("auth lock poisoned").token.clone()
    }

    pub(crate) fn register_cache_table(&self, table: &'static str) {
        self.cache_tables
            .write()
            .expect("cache tables lock poisoned")
            .insert(table);
    }

    pub(crate) async fn cache_session(&self, table: &'static str) -> ApiResult<CacheSession<'_>> {
        self.register_cache_table(table);
        let mut cache = self.cache.lock().await;
        let generation = {
            let auth = self.auth.read().expect("auth lock poisoned");
            if auth.token.as_ref().is_none_or(|token| token.is_empty()) {
                return Err(ApiError::new(
                    "not_authenticated",
                    "authentication token is missing",
                ));
            }
            auth.generation
        };
        if cache.generation != Some(generation) {
            let tables = self
                .cache_tables
                .read()
                .expect("cache tables lock poisoned")
                .clone();
            let sql = tables
                .iter()
                .map(|table| format!("DELETE FROM \"{}\";", table.replace('"', "\"\"")))
                .collect::<String>();
            // Clear all cached resources together, including related models, while
            // preserving ordinary local resources and the migration history.
            self.db
                .raw_transaction(move |connection| {
                    connection.execute_batch(&format!("PRAGMA defer_foreign_keys = ON;{sql}"))
                })
                .await?;
            cache.generation = Some(generation);
        }
        Ok(CacheSession {
            _guard: cache,
            generation,
        })
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
