mod common;

use che_orm::{Database, Model, SchemaSet};
use che_tauri::{
    ApiAction, ApiRequest, AppConfig, AppModule, AppState, DatabaseConfig, Field, FilterSet,
    InstalledApps, ModelSerializer, ModuleContext, TauriApi,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Model, serde::Serialize)]
#[orm(table = "lifecycle_notes", index("title"))]
struct Note {
    #[orm(primary_key)]
    id: i64,
    title: String,
}
static FIELDS: &[Field] = &[Field::new("id").read_only(), Field::new("title")];

async fn state() -> AppState {
    AppState::new(
        AppConfig {
            database: DatabaseConfig {
                url: ":memory:".into(),
            },
            remote: None,
        },
        Database::connect_in_memory().await.unwrap(),
    )
}

struct Tracked {
    name: &'static str,
    log: Arc<Mutex<Vec<String>>>,
}
impl Tracked {
    fn record(&self, stage: &str) {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:{stage}", self.name));
    }
}
impl AppModule for Tracked {
    fn name(&self) -> &'static str {
        self.name
    }
    fn schema(&self) -> SchemaSet {
        self.record("schema");
        SchemaSet::new()
    }
    fn init(&self, _ctx: &mut ModuleContext) {
        self.record("init");
    }
    fn subscribe(&self, _state: &AppState) {
        self.record("subscribe");
    }
    fn start(&self, _state: &AppState) {
        self.record("start");
    }
}

#[tokio::test]
async fn install_is_deferred_and_build_runs_registration_before_all_start_hooks() {
    let log = Arc::new(Mutex::new(vec![]));
    let apps = InstalledApps::new()
        .add(Tracked {
            name: "a",
            log: log.clone(),
        })
        .add(Tracked {
            name: "b",
            log: log.clone(),
        });
    let api = TauriApi::new(state().await).install(apps);
    assert!(log.lock().unwrap().is_empty());
    let _api = api.build().await.unwrap();
    assert_eq!(
        *log.lock().unwrap(),
        [
            "a:schema",
            "a:init",
            "b:schema",
            "b:init",
            "a:subscribe",
            "b:subscribe",
            "a:start",
            "b:start"
        ]
    );
}

struct Notes;
impl AppModule for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }
    fn schema(&self) -> SchemaSet {
        SchemaSet::new().model::<Note>()
    }
    fn init(&self, ctx: &mut ModuleContext) {
        ctx.resource::<Note>("notes", ModelSerializer::new(FIELDS), FilterSet::new(&[]));
    }
}

#[tokio::test]
async fn startup_never_creates_tables_and_explicit_migrations_enable_crud() {
    let apps = InstalledApps::new().add(Notes);
    let state = state().await;
    let api = TauriApi::new(state.clone())
        .install(apps.clone())
        .build()
        .await
        .unwrap();
    let objects = state
        .db()
        .query_sql(
            "SELECT name FROM sqlite_master WHERE name = 'lifecycle_notes'",
            vec![],
        )
        .await
        .unwrap();
    assert!(objects.is_empty());
    let request = || ApiRequest {
        resource: "notes".into(),
        action: ApiAction::Create,
        id: None,
        params: Default::default(),
        payload: Some(json!({"title":"migrated"})),
    };
    assert!(api.dispatch(request()).await.is_err());
    common::migrate(state.db(), &apps).await;
    common::migrate(state.db(), &apps).await;
    assert_eq!(api.dispatch(request()).await.unwrap()["title"], "migrated");
    let rows = state
        .db()
        .query_sql(
            "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='lifecycle_notes'",
            vec![],
        )
        .await
        .unwrap();
    assert!(!rows.is_empty());
}

struct SchemaOnly;
impl AppModule for SchemaOnly {
    fn name(&self) -> &'static str {
        "notes"
    }
    fn schema(&self) -> SchemaSet {
        SchemaSet::new().model::<Note>()
    }
    fn init(&self, _ctx: &mut ModuleContext) {
        panic!("schema discovery must not run init");
    }
}

#[test]
fn schema_and_migration_discovery_never_call_init() {
    let apps = InstalledApps::new().add(SchemaOnly);
    assert_eq!(apps.schema().tables()[0].name, "lifecycle_notes");
    assert_eq!(
        apps.migration_state().unwrap().tables()[0].name,
        "lifecycle_notes"
    );
}

struct MissingSchema {
    started: Arc<Mutex<bool>>,
}
impl AppModule for MissingSchema {
    fn name(&self) -> &'static str {
        "missing"
    }
    fn schema(&self) -> SchemaSet {
        SchemaSet::new()
    }
    fn init(&self, ctx: &mut ModuleContext) {
        ctx.resource::<Note>("notes", ModelSerializer::new(FIELDS), FilterSet::new(&[]));
    }
    fn start(&self, _state: &AppState) {
        *self.started.lock().unwrap() = true;
    }
}

#[tokio::test]
async fn missing_schema_fails_registration_before_start() {
    let started = Arc::new(Mutex::new(false));
    let error = TauriApi::new(state().await)
        .install(InstalledApps::new().add(MissingSchema {
            started: started.clone(),
        }))
        .build()
        .await
        .unwrap_err();
    assert!(error.detail.contains("AppModule::schema"));
    assert!(!*started.lock().unwrap());
}

#[cfg(feature = "sync")]
mod sync {
    use super::*;

    #[derive(Model, serde::Serialize)]
    #[orm(table = "sync_notes", sync, sync_name = "notes.note")]
    struct SyncNote {
        #[orm(primary_key)]
        id: che_orm::Uuid,
        title: String,
    }
    struct SyncNotes;
    impl AppModule for SyncNotes {
        fn name(&self) -> &'static str {
            "notes"
        }
        fn schema(&self) -> SchemaSet {
            SchemaSet::new().client_model::<SyncNote>()
        }
        fn init(&self, _ctx: &mut ModuleContext) {
            panic!("sync discovery must not run init");
        }
    }
    #[test]
    fn sync_registry_comes_from_schema_without_runtime_initialization() {
        let apps = InstalledApps::new().add(SyncNotes);
        apps.sync_registry().unwrap();
        assert!(
            apps.migration_state()
                .unwrap()
                .tables()
                .iter()
                .any(|t| t.name == "_sync_server_sync_notes")
        );
    }

    struct Overlap {
        cached: bool,
    }
    impl AppModule for Overlap {
        fn name(&self) -> &'static str {
            "notes"
        }
        fn schema(&self) -> SchemaSet {
            SchemaSet::new().client_model::<SyncNote>()
        }
        fn init(&self, ctx: &mut ModuleContext) {
            if self.cached {
                ctx.cached_mapped_remote_resource::<SyncNote>(
                    "notes",
                    "/notes/",
                    ModelSerializer::new(FIELDS),
                    FilterSet::new(&[]),
                );
            } else {
                ctx.resource::<SyncNote>(
                    "notes",
                    ModelSerializer::new(FIELDS),
                    FilterSet::new(&[]),
                );
            }
        }
    }
    #[tokio::test]
    async fn ordinary_local_and_cached_resources_cannot_own_sync_tables() {
        for cached in [true, false] {
            let error = TauriApi::new(state().await)
                .install(InstalledApps::new().add(Overlap { cached }))
                .build()
                .await
                .unwrap_err();
            assert!(error.detail.contains("sync table sync_notes"));
        }
    }
    #[derive(Model, serde::Serialize)]
    #[orm(table = "_sync_object_access")]
    struct AccessAlias {
        #[orm(primary_key)]
        id: i64,
        title: String,
    }
    struct InternalAlias {
        cached: bool,
    }
    impl AppModule for InternalAlias {
        fn name(&self) -> &'static str {
            "invalid_cache"
        }
        fn schema(&self) -> SchemaSet {
            SchemaSet::new().model::<AccessAlias>()
        }
        fn init(&self, ctx: &mut ModuleContext) {
            if self.cached {
                ctx.cached_mapped_remote_resource::<AccessAlias>(
                    "access",
                    "/access/",
                    ModelSerializer::new(FIELDS),
                    FilterSet::new(&[]),
                );
            } else {
                ctx.resource::<AccessAlias>(
                    "access",
                    ModelSerializer::new(FIELDS),
                    FilterSet::new(&[]),
                );
            }
        }
    }
    #[tokio::test]
    async fn object_delivery_infrastructure_cannot_be_registered_as_a_cache_or_resource() {
        for cached in [false, true] {
            let error = TauriApi::new(state().await)
                .install(InstalledApps::new().add(InternalAlias { cached }))
                .build()
                .await
                .unwrap_err();
            assert!(error.detail.contains("sync table _sync_object_access"));
        }
    }
}
