use che_orm::Database;
use che_tauri::{SyncRequest, SyncResponse};
use todo_sync_model::Todo;
use todo_sync_server::shared;

#[tokio::test]
async fn two_accounts_share_one_todo_then_revoke_keep_draft_and_regrant() {
    let db = Database::connect_in_memory().await.unwrap();
    shared::migrate(&db).await.unwrap();
    shared::migrate(&db).await.unwrap();
    let state = shared::state(db.clone()).await.unwrap();
    let router = che_rest::Server::new(state)
        .install(shared::apps())
        .build()
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let local_a = Database::connect_in_memory().await.unwrap();
    let local_b = Database::connect_in_memory().await.unwrap();
    for local in [&local_a, &local_b] {
        todo_sync_client::migrate_objects(local).await.unwrap();
        todo_sync_client::migrate_objects(local).await.unwrap();
    }
    let a = todo_sync_client::open_objects(local_a, &base, "todo-alice", "alice")
        .await
        .unwrap();
    let b = todo_sync_client::open_objects(local_b.clone(), &base, "todo-bob", "bob")
        .await
        .unwrap();
    a.pull().await.unwrap();
    b.pull().await.unwrap();
    todo_sync_client::create(&a, "Shared Todo").await.unwrap();
    assert_eq!(a.flush().await.unwrap().accepted_operations, 1);
    let id = todo_sync_client::list(&a).await.unwrap()[0].id;
    b.pull().await.unwrap();
    assert!(todo_sync_client::list(&b).await.unwrap().is_empty());
    shared::share(&db, id, "bob", true).await.unwrap();
    b.pull().await.unwrap();
    assert_eq!(todo_sync_client::list(&b).await.unwrap()[0].id, id);
    todo_sync_client::update(&b, id, Some("Bob's offline draft".into()), None)
        .await
        .unwrap();
    shared::share(&db, id, "bob", false).await.unwrap();
    assert!(b.pull().await.unwrap().blocked);
    assert!(todo_sync_client::list(&b).await.unwrap().is_empty());
    assert_eq!(
        a.database().get::<Todo>(id).await.unwrap().unwrap().title,
        "Shared Todo"
    );
    let access = b.dispatch(SyncRequest::ObjectAccess).await.unwrap();
    assert!(
        matches!(access,SyncResponse::ObjectAccess(v) if v.len()==1 && v[0].status=="revoked" && v[0].has_draft)
    );
    let draft = b
        .dispatch(SyncRequest::RetainedDraft {
            model: "todo.task".into(),
            id,
        })
        .await
        .unwrap();
    assert!(matches!(draft, SyncResponse::RetainedDraft(Some(_))));
    drop(b);
    let b = todo_sync_client::open_objects(local_b, &base, "todo-bob", "bob")
        .await
        .unwrap();
    assert!(todo_sync_client::list(&b).await.unwrap().is_empty());
    shared::share(&db, id, "bob", true).await.unwrap();
    b.pull().await.unwrap();
    assert_eq!(b.flush().await.unwrap().accepted_operations, 0);
    b.dispatch(SyncRequest::ResumeObject {
        model: "todo.task".into(),
        id,
    })
    .await
    .unwrap();
    assert_eq!(b.flush().await.unwrap().accepted_operations, 1);
    a.pull().await.unwrap();
    assert_eq!(
        a.database().get::<Todo>(id).await.unwrap().unwrap().title,
        "Bob's offline draft"
    );
    let rows = db
        .query_sql(
            "SELECT revision FROM todos WHERE id=$1",
            vec![che_orm::DatabaseValue::Uuid(id)],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<i64>(0).unwrap(), 2);
    server.abort();
}

#[test]
fn frozen_object_migrations_match_declared_server_and_client_infrastructure() {
    let apps = shared::apps();
    let history = apps.iter().flat_map(|app| app.migrations()).collect();
    let frozen = che_orm::MigrationGraph::new(history)
        .unwrap()
        .replay(che_orm::ProjectState::default())
        .unwrap();
    let desired = apps.migration_state().unwrap();
    assert!(
        frozen.diff(&desired).unwrap().is_empty(),
        "{:?}",
        frozen.diff(&desired)
    );
    let client_apps = todo_sync_client::object_apps();
    let client = client_apps.migration_state().unwrap();
    let history = client_apps
        .iter()
        .flat_map(|app| app.migrations())
        .collect();
    let frozen = che_orm::MigrationGraph::new(history)
        .unwrap()
        .replay(che_orm::ProjectState::default())
        .unwrap();
    assert!(
        frozen.diff(&client).unwrap().is_empty(),
        "{:?}",
        frozen.diff(&client)
    );
    assert!(
        client
            .models()
            .iter()
            .any(|m| m.table.name == "_sync_object_access" && m.key.app == "che_object_sync")
    );
}
