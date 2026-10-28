//! Protocol-2 synchronization exposed through a dedicated Tauri state/command.
//!
//! Apply compiled client migrations before opening. Manage one instance per
//! database and clone it to share engine locks. The application supplies the
//! transport, credentials and scheduling; no background task starts implicitly.
use std::sync::Arc;

use async_trait::async_trait;
use che_orm::{Database, Uuid};
use che_sync::{
    ModelLogClient, ModelLogEngine, ModelLogTransport, Registry, Resolution, SyncModel, SyncReport,
    protocol::{Conflict, Record},
};
use serde::{Deserialize, Serialize};

pub mod schema;

use crate::{ApiResult, error::ApiError};

/// Frontend commands are independent of the i64 resource CRUD request format.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SyncRequest {
    Pull,
    Flush,
    SyncOnce,
    Conflicts,
    ObjectAccess,
    RetainedDraft {
        model: String,
        id: Uuid,
    },
    ResumeObject {
        model: String,
        id: Uuid,
    },
    DiscardRetainedDraft {
        model: String,
        id: Uuid,
    },
    Resolve {
        model: String,
        id: Uuid,
        resolution: SyncResolution,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum SyncResolution {
    Local,
    Remote,
    /// Typed protocol values preserve SQL NULL, JSON null and i64 precision.
    Record {
        #[serde(deserialize_with = "Deserialize::deserialize")]
        record: Option<Record>,
    },
}

impl From<SyncResolution> for Resolution {
    fn from(value: SyncResolution) -> Self {
        match value {
            SyncResolution::Local => Self::Local,
            SyncResolution::Remote => Self::Remote,
            SyncResolution::Record { record } => Self::Record(record),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum SyncResponse {
    Report(SyncStatus),
    Conflicts(Vec<Conflict>),
    ObjectAccess(Vec<che_sync::object_log::ObjectStatus>),
    RetainedDraft(Option<Record>),
    Resolved,
}

/// One bounded engine step. Inspect `blocked`/`conflicts` before scheduling
/// another step, and keep pulling while `has_more` is true.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SyncStatus {
    pub accepted_operations: usize,
    pub rejected_operations: usize,
    pub received_changes: usize,
    pub conflicts: usize,
    pub has_more: bool,
    pub blocked: bool,
}

impl From<SyncReport> for SyncStatus {
    fn from(value: SyncReport) -> Self {
        Self {
            accepted_operations: value.accepted_operations,
            rejected_operations: value.rejected_operations,
            received_changes: value.received_changes,
            conflicts: value.conflicts,
            has_more: value.has_more,
            blocked: value.blocked,
        }
    }
}

#[async_trait]
trait Engine: Send + Sync {
    async fn pull(&self) -> ApiResult<SyncStatus>;
    async fn flush(&self) -> ApiResult<SyncStatus>;
    async fn sync_once(&self) -> ApiResult<SyncStatus>;
}

#[async_trait]
impl<T: ModelLogTransport> Engine for ModelLogEngine<T> {
    async fn pull(&self) -> ApiResult<SyncStatus> {
        Ok(ModelLogEngine::pull(self).await?.into())
    }
    async fn flush(&self) -> ApiResult<SyncStatus> {
        Ok(ModelLogEngine::flush(self).await?.into())
    }
    async fn sync_once(&self) -> ApiResult<SyncStatus> {
        Ok(ModelLogEngine::sync_once(self).await?.into())
    }
}

/// Type-erased transport allows one non-generic Tauri managed state.
/// This database is durable offline data, independent of remote response caches.
#[derive(Clone)]
pub struct TauriSync {
    client: ModelLogClient,
    engine: Arc<dyn Engine>,
}

impl TauriSync {
    pub async fn open<T: ModelLogTransport + 'static>(
        database: Database,
        registry: Registry,
        transport: T,
    ) -> ApiResult<Self> {
        Ok(Self::new(
            ModelLogClient::open(database, registry).await?,
            transport,
        ))
    }

    /// Explicit object-delivery mode. Existing v2 databases are not upgraded here.
    pub async fn open_objects<T: ModelLogTransport + 'static>(
        database: Database,
        registry: Registry,
        server: impl Into<String>,
        account: impl Into<String>,
        transport: T,
    ) -> ApiResult<Self> {
        Ok(Self::new(
            ModelLogClient::open_objects(database, registry, server, account).await?,
            transport,
        ))
    }

    pub fn new<T: ModelLogTransport + 'static>(client: ModelLogClient, transport: T) -> Self {
        Self::from_engine(client.engine(transport))
    }

    /// Accept an engine configured with `with_page_size`, retaining its client.
    pub fn from_engine<T: ModelLogTransport + 'static>(engine: ModelLogEngine<T>) -> Self {
        Self {
            client: engine.client().clone(),
            engine: Arc::new(engine),
        }
    }

    pub fn client(&self) -> &ModelLogClient {
        &self.client
    }

    pub fn database(&self) -> &Database {
        self.client.database()
    }

    /// Use from application-owned UUID CRUD commands, including offline.
    pub async fn save<M: SyncModel>(&self, model: &M) -> ApiResult<()> {
        Ok(self.client.save(model).await?)
    }

    pub async fn delete<M: SyncModel>(&self, id: Uuid) -> ApiResult<()> {
        Ok(self.client.delete::<M>(id).await?)
    }

    pub async fn pull(&self) -> ApiResult<SyncStatus> {
        self.engine.pull().await
    }

    pub async fn flush(&self) -> ApiResult<SyncStatus> {
        self.engine.flush().await
    }

    pub async fn sync_once(&self) -> ApiResult<SyncStatus> {
        self.engine.sync_once().await
    }

    pub async fn conflicts(&self) -> ApiResult<Vec<Conflict>> {
        Ok(self.client.conflicts().await?)
    }

    pub async fn resolve(
        &self,
        model: &str,
        id: Uuid,
        resolution: SyncResolution,
    ) -> ApiResult<()> {
        Ok(self.client.resolve(model, id, resolution.into()).await?)
    }

    pub async fn dispatch(&self, request: SyncRequest) -> ApiResult<SyncResponse> {
        match request {
            SyncRequest::Pull => Ok(SyncResponse::Report(self.pull().await?)),
            SyncRequest::Flush => Ok(SyncResponse::Report(self.flush().await?)),
            SyncRequest::SyncOnce => Ok(SyncResponse::Report(self.sync_once().await?)),
            SyncRequest::Conflicts => Ok(SyncResponse::Conflicts(self.conflicts().await?)),
            SyncRequest::ObjectAccess => Ok(SyncResponse::ObjectAccess(
                self.client.object_status().await?,
            )),
            SyncRequest::RetainedDraft { model, id } => Ok(SyncResponse::RetainedDraft(
                self.client.retained_draft(&model, id).await?,
            )),
            SyncRequest::ResumeObject { model, id } => {
                self.client.resume_object(&model, id).await?;
                Ok(SyncResponse::Resolved)
            }
            SyncRequest::DiscardRetainedDraft { model, id } => {
                self.client.discard_retained_draft(&model, id).await?;
                Ok(SyncResponse::Resolved)
            }
            SyncRequest::Resolve {
                model,
                id,
                resolution,
            } => {
                self.resolve(&model, id, resolution).await?;
                Ok(SyncResponse::Resolved)
            }
        }
    }
}

#[tauri::command]
pub async fn che_model_sync(
    sync: tauri::State<'_, TauriSync>,
    request: SyncRequest,
) -> Result<SyncResponse, ApiError> {
    sync.dispatch(request).await
}

pub(crate) const TYPESCRIPT_CLIENT: &str = include_str!("sync_client.ts");

/// Schema-only module for protocol-3 client infrastructure. Include alongside
/// application modules when discovering desired compiled migration state.
pub struct ObjectSyncSchema;
impl crate::AppModule for ObjectSyncSchema {
    fn name(&self) -> &'static str {
        "che_object_sync"
    }
    fn schema(&self) -> che_orm::SchemaSet {
        che_sync::object_log::schema(che_sync::SchemaRole::Client)
    }
    fn init(&self, _: &mut crate::ModuleContext) {}
}
