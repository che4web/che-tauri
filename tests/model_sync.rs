#![cfg(feature = "sync")]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use che_tauri::{
    AppModule, InstalledApps, ModuleContext, SyncRequest, SyncResponse, TauriSync,
    che_orm::{
        Database, Migration, MigrationGraph, MigrationId, MigrationOperation, Model, OrmError,
        ProjectState, SchemaSet, Uuid, ValidatedWrite,
    },
    che_sync::{
        AllowAll, ModelLogClient, ModelLogTransport, Registry, SyncServer, TransportFuture,
        model_log::{ModelChanges, ModelState, ModelWrite, ModelWriteResult},
    },
};
use serde_json::{Value, json};

#[derive(Debug, Model)]
#[orm(table = "notes", sync, sync_name = "notes.note")]
struct Note {
    #[orm(primary_key)]
    id: Uuid,
    title: String,
    number: i64,
}

struct Notes;
impl AppModule for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }
    fn schema(&self) -> SchemaSet {
        SchemaSet::new().client_model::<Note>()
    }
    fn init(&self, _ctx: &mut ModuleContext) {}
}

async fn database(client: bool) -> (Database, Registry) {
    let schema = if client {
        SchemaSet::new().client_model::<Note>()
    } else {
        SchemaSet::new().server_model::<Note>()
    };
    let state = ProjectState::from_app_schemas([("notes", &schema)]).unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    db.migrate(
        MigrationGraph::new(vec![Migration::new(
            MigrationId {
                app: "notes".into(),
                name: "0001".into(),
            },
            vec![],
            state
                .models()
                .iter()
                .cloned()
                .map(|model| MigrationOperation::CreateModel { model })
                .collect(),
        )])
        .unwrap(),
    )
    .await
    .unwrap();
    let registry = if client {
        InstalledApps::new().add(Notes).sync_registry().unwrap()
    } else {
        Registry::from_schema(&schema).unwrap()
    };
    (db, registry)
}

#[derive(Clone)]
struct Transport {
    server: SyncServer,
    offline: Arc<AtomicBool>,
    reject: Arc<AtomicBool>,
    delivered: Arc<Mutex<Vec<Value>>>,
    values: Arc<Mutex<Option<serde_json::Map<String, Value>>>>,
}
impl ModelLogTransport for Transport {
    fn id(&self) -> &'static str {
        "test.native.v1"
    }
    fn write<'a>(
        &'a self,
        write: &'a ModelWrite,
        payload: &'a Value,
    ) -> TransportFuture<'a, ModelWriteResult> {
        Box::pin(async move {
            self.delivered.lock().unwrap().push(payload.clone());
            if self.offline.load(Ordering::SeqCst) {
                return Err(OrmError::Interaction("offline".into()));
            }
            if self.reject.load(Ordering::SeqCst) {
                return Ok(ModelWriteResult::Rejected {
                    code: "domain".into(),
                    reason: "rejected".into(),
                });
            }
            let mut values = self.values.lock().unwrap();
            if let Some(changes) = &write.changes {
                values.get_or_insert_default().extend(changes.clone());
                values
                    .as_mut()
                    .unwrap()
                    .insert("id".into(), json!(write.record_id));
            } else {
                *values = None;
            }
            Ok(ModelWriteResult::Accepted(vec![ModelState {
                model: write.model.clone(),
                record_id: write.record_id,
                revision: write.base_revision + 1,
                schema_version: write.schema_version,
                values: values.clone(),
                sql_null_fields: vec![],
                updated_at: Some(che_tauri::che_orm::time::OffsetDateTime::now_utc()),
                deleted_at: write
                    .changes
                    .is_none()
                    .then(che_tauri::che_orm::time::OffsetDateTime::now_utc),
            }]))
        })
    }
    fn changes<'a>(
        &'a self,
        model: &'a str,
        after: i64,
        limit: u32,
    ) -> TransportFuture<'a, ModelChanges> {
        Box::pin(async move {
            assert_eq!(model, "notes.note");
            self.server.changes::<Note>(after, limit).await
        })
    }
}

async fn transport() -> Transport {
    let (db, registry) = database(false).await;
    Transport {
        server: SyncServer::open_model_logs(db, registry, "user", Arc::new(AllowAll))
            .await
            .unwrap(),
        offline: Arc::new(AtomicBool::new(false)),
        reject: Arc::new(AtomicBool::new(false)),
        delivered: Arc::default(),
        values: Arc::default(),
    }
}

fn request(value: Value) -> SyncRequest {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn offline_edits_replay_after_reopen_and_conflicts_resolve_through_invoke() {
    let (db, registry) = database(true).await;
    let transport = transport().await;
    let sync = TauriSync::open(db.clone(), registry.clone(), transport.clone())
        .await
        .unwrap();
    sync.pull().await.unwrap();
    let mut note = Note {
        id: Uuid::now_v7(),
        title: String::new(),
        number: i64::MAX,
    };
    for n in 0..10 {
        note.title = format!("edit {n}");
        sync.save(&note).await.unwrap();
    }
    transport.offline.store(true, Ordering::SeqCst);
    assert!(
        sync.dispatch(request(json!({"action":"flush"})))
            .await
            .is_err()
    );
    drop(sync);
    let sync = TauriSync::open(db, registry, transport.clone())
        .await
        .unwrap();
    transport.offline.store(false, Ordering::SeqCst);
    let response = sync
        .dispatch(request(json!({"action":"flush"})))
        .await
        .unwrap();
    let response = serde_json::to_value(response).unwrap();
    assert_eq!(response["kind"], "report");
    assert_eq!(response["data"]["accepted_operations"], 1);
    let submitted = transport.delivered.lock().unwrap().clone();
    assert_eq!(submitted.len(), 2);
    assert_eq!(submitted[0], submitted[1]);
    assert_eq!(submitted[0]["changes"]["title"], "edit 9");
    assert_eq!(submitted[0]["changes"]["number"], i64::MAX.to_string());

    note.title = "invalid edit".into();
    sync.save(&note).await.unwrap();
    transport.reject.store(true, Ordering::SeqCst);
    assert_eq!(sync.flush().await.unwrap().rejected_operations, 1);
    let conflicts = sync
        .dispatch(request(json!({"action":"conflicts"})))
        .await
        .unwrap();
    let conflicts = serde_json::to_value(conflicts).unwrap();
    assert_eq!(conflicts["data"].as_array().unwrap().len(), 1);
    assert_eq!(conflicts["data"][0]["remote"]["revision"], "1");
    assert_eq!(
        conflicts["data"][0]["local"]["number"],
        json!({"type":"integer","value":i64::MAX.to_string()})
    );
    let response = sync
        .dispatch(request(
            json!({"action":"resolve", "model":"notes.note", "id":note.id,
        "resolution":{"strategy":"remote"}}),
        ))
        .await
        .unwrap();
    assert!(matches!(response, SyncResponse::Resolved));
    assert!(sync.conflicts().await.unwrap().is_empty());
    assert_eq!(
        sync.database()
            .get::<Note>(note.id)
            .await
            .unwrap()
            .unwrap()
            .title,
        "edit 9"
    );
    transport.reject.store(false, Ordering::SeqCst);
    sync.delete::<Note>(note.id).await.unwrap();
    assert_eq!(sync.flush().await.unwrap().accepted_operations, 1);
    assert!(
        sync.database()
            .get::<Note>(note.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn downloads_are_paginated_and_sync_once_uses_the_same_client() {
    let transport = transport().await;
    let mut ids = vec![];
    for title in ["one", "two"] {
        let id = Uuid::now_v7();
        ids.push(id);
        transport
            .server
            .write(ValidatedWrite::Create(
                Note::insert()
                    .set(Note::ID, id)
                    .set(Note::TITLE, title)
                    .set(Note::NUMBER, 42),
            ))
            .await
            .unwrap();
    }
    let (db, registry) = database(true).await;
    let client = ModelLogClient::open(db, registry).await.unwrap();
    let sync = TauriSync::from_engine(client.engine(transport).with_page_size(1).unwrap());
    let first = sync
        .dispatch(request(json!({"action":"pull"})))
        .await
        .unwrap();
    let first = serde_json::to_value(first).unwrap();
    assert_eq!(first["data"]["received_changes"], 1);
    assert_eq!(first["data"]["has_more"], true);
    let last = sync
        .dispatch(request(json!({"action":"sync_once"})))
        .await
        .unwrap();
    let last = serde_json::to_value(last).unwrap();
    assert_eq!(last["data"]["received_changes"], 1);
    assert_eq!(last["data"]["has_more"], false);
    for id in ids {
        assert!(sync.database().get::<Note>(id).await.unwrap().is_some());
    }
    assert_eq!(sync.client().cursor::<Note>().await.unwrap(), 2);
}

#[test]
fn resolution_requires_explicit_record_and_valid_uuid() {
    let id = Uuid::now_v7();
    for value in [
        json!({"action":"resolve", "model":"notes.note", "id":"invalid", "resolution":{"strategy":"remote"}}),
        json!({"action":"resolve", "model":"notes.note", "id":id, "resolution":{"strategy":"record"}}),
    ] {
        assert!(serde_json::from_value::<SyncRequest>(value).is_err());
    }
    let deletion = request(json!({"action":"resolve", "model":"notes.note", "id":id,
        "resolution":{"strategy":"record", "record":null}}));
    assert!(matches!(
        deletion,
        SyncRequest::Resolve {
            resolution: che_tauri::SyncResolution::Record { record: None },
            ..
        }
    ));
}
