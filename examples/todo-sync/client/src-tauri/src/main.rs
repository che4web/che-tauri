use che_orm::Uuid;
use che_tauri::{ApiResult, TauriSync};
use todo_sync_model::Todo;

#[tauri::command]
async fn todo_list(sync: tauri::State<'_, TauriSync>) -> ApiResult<Vec<Todo>> {
    todo_sync_client::list(&sync).await
}
#[tauri::command]
async fn todo_create(sync: tauri::State<'_, TauriSync>, title: String) -> ApiResult<()> {
    todo_sync_client::create(&sync, &title).await
}
#[tauri::command]
async fn todo_update(
    sync: tauri::State<'_, TauriSync>,
    id: Uuid,
    title: Option<String>,
    done: Option<bool>,
) -> ApiResult<()> {
    todo_sync_client::update(&sync, id, title, done).await
}
#[tauri::command]
async fn todo_delete(sync: tauri::State<'_, TauriSync>, id: Uuid) -> ApiResult<()> {
    todo_sync_client::remove(&sync, id).await
}

#[tauri::command]
fn todo_object_mode(sync: tauri::State<'_, TauriSync>) -> bool {
    sync.client().is_object_sync()
}

fn main() {
    let sync = tauri::async_runtime::block_on(async {
        let db = che_orm::Database::connect(todo_sync_client::database_path()).await?;
        let base =
            std::env::var("TODO_SYNC_URL").unwrap_or_else(|_| "http://127.0.0.1:3007/api".into());
        let token = std::env::var("TODO_SYNC_TOKEN").unwrap_or_else(|_| "todo-demo-token".into());
        match std::env::var("TODO_OBJECT_ACCOUNT") {
            Ok(account) => todo_sync_client::open_objects(db, &base, &token, &account).await,
            Err(_) => todo_sync_client::open(db, &base, &token).await,
        }
    })
    .expect("Run todo-client-db migrate first; failed to open the offline client");
    tauri::Builder::default()
        .manage(sync)
        .invoke_handler(tauri::generate_handler![
            che_tauri::che_model_sync,
            todo_list,
            todo_object_mode,
            todo_create,
            todo_update,
            todo_delete
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Todo client");
}
