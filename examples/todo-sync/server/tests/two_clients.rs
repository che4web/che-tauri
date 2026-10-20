use che_orm::Database;
use serde_json::json;
use todo_sync_client as client;
use todo_sync_model::Todo;
use todo_sync_server as server;

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("todo-sync-{}", che_orm::Uuid::now_v7()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    async fn db(&self, name: &str) -> Database {
        Database::connect(self.0.join(name).to_string_lossy().to_string())
            .await
            .unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn settle(sync: &che_tauri::TauriSync) {
    for _ in 0..20 {
        let report = sync.sync_once().await.unwrap();
        assert!(!report.blocked, "{report:?}");
        assert_eq!(report.conflicts, 0);
        assert_eq!(report.rejected_operations, 0);
        if !report.has_more && report.received_changes == 0 && report.accepted_operations == 0 {
            return;
        }
    }
    panic!("sync failed to settle");
}

#[tokio::test]
async fn offline_restart_two_devices_merge_conflict_and_delete_over_http() {
    let temp = Temp::new();
    let server_db = temp.db("server.sqlite").await;
    server::migrate(&server_db).await.unwrap();
    server::migrate(&server_db).await.unwrap();
    let router = che_rest::Server::new(server::state(server_db, server::DEMO_TOKEN).await.unwrap())
        .install(server::apps())
        .build()
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api", listener.local_addr().unwrap());

    let db = temp.db("a.sqlite").await;
    client::migrate(&db).await.unwrap();
    let a = client::open(db, &base, server::DEMO_TOKEN).await.unwrap();
    // The listener isn't serving yet: all CRUD is local, and failed delivery is harmless.
    assert!(client::create(&a, "  ").await.is_err());
    client::create(&a, "Offline task").await.unwrap();
    let id = client::list(&a).await.unwrap()[0].id;
    client::update(&a, id, Some("Edited offline".into()), None)
        .await
        .unwrap();
    drop(a);
    let db = temp.db("a.sqlite").await;
    client::migrate(&db).await.unwrap(); // repeated migration retains offline edits
    let a = client::open(db, &base, server::DEMO_TOKEN).await.unwrap();
    assert_eq!(client::list(&a).await.unwrap()[0].title, "Edited offline");

    let serving = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let db = temp.db("b.sqlite").await;
    client::migrate(&db).await.unwrap();
    let b = client::open(db, &base, server::DEMO_TOKEN).await.unwrap();
    settle(&a).await;
    settle(&b).await;
    assert_eq!(client::list(&b).await.unwrap()[0].title, "Edited offline");

    // Independent fields merge without intervention.
    client::update(&a, id, Some("From device A".into()), None)
        .await
        .unwrap();
    client::update(&b, id, None, Some(true)).await.unwrap();
    settle(&a).await;
    settle(&b).await;
    settle(&a).await;
    let merged = a.database().get::<Todo>(id).await.unwrap().unwrap();
    assert_eq!(merged.title, "From device A");
    assert!(merged.done);

    // Competing edits to the same field are exposed through the invoke contract.
    client::update(&a, id, Some("Server version".into()), None)
        .await
        .unwrap();
    client::update(&b, id, Some("Local version".into()), None)
        .await
        .unwrap();
    settle(&a).await;
    b.pull().await.unwrap();
    let conflicts = b.conflicts().await.unwrap();
    assert_eq!(conflicts.len(), 1);
    let request = serde_json::from_value(json!({"action":"resolve", "model":"todo.task", "id":id,
        "resolution":{"strategy":"remote"}}))
    .unwrap();
    b.dispatch(request).await.unwrap();
    settle(&b).await;
    assert_eq!(client::list(&b).await.unwrap()[0].title, "Server version");

    client::remove(&a, id).await.unwrap();
    settle(&a).await;
    settle(&b).await;
    assert!(client::list(&b).await.unwrap().is_empty());

    let db = temp.db("unauthorized.sqlite").await;
    client::migrate(&db).await.unwrap();
    let unauthorized = client::open(db, &base, "wrong-token").await.unwrap();
    assert!(unauthorized.pull().await.is_err());
    serving.abort();
}
