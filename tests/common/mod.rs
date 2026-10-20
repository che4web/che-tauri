use che_orm::{Database, Migration, MigrationGraph, MigrationId, MigrationOperation};
use che_tauri::InstalledApps;

pub async fn migrate(db: &Database, apps: &InstalledApps) {
    let state = apps.migration_state().unwrap();
    let operations = state
        .models()
        .iter()
        .cloned()
        .map(|model| MigrationOperation::CreateModel { model })
        .collect();
    db.migrate(
        MigrationGraph::new(vec![Migration::new(
            MigrationId {
                app: "test".into(),
                name: "0001".into(),
            },
            vec![],
            operations,
        )])
        .unwrap(),
    )
    .await
    .unwrap();
}
