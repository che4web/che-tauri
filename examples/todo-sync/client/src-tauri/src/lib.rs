use che_orm::{Database, MigrationGraph, Uuid};
use che_rest::sync_http::{RestModelTransport, RestResource};
use che_tauri::{ApiError, ApiResult, AppModule, InstalledApps, ModuleContext, TauriSync};
use std::time::Duration;
use todo_sync_model::{Todo, TodoSerializer};

mod initial;

struct TodoApp;
impl AppModule for TodoApp {
    fn name(&self) -> &'static str {
        "todo"
    }
    fn init(&self, ctx: &mut ModuleContext) {
        ctx.sync_model::<Todo>();
    }
    fn migrations(&self) -> Vec<che_orm::Migration> {
        vec![initial::migration()]
    }
}

pub fn apps() -> InstalledApps {
    InstalledApps::new().add(TodoApp)
}

pub async fn migrate(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    db.migrate(MigrationGraph::new(vec![initial::migration()])?)
        .await?;
    Ok(())
}

pub fn database_path() -> String {
    std::env::var("TODO_CLIENT_DB").unwrap_or_else(|_| "client-a.sqlite".into())
}

pub async fn open(db: Database, base: &str, token: &str) -> ApiResult<TauriSync> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Token {token}")
            .parse()
            .map_err(|_| ApiError::bad_request("invalid token"))?,
    );
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .default_headers(headers)
        .build()
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let transport = RestModelTransport::new(
        base,
        http,
        vec![RestResource::new::<Todo, TodoSerializer>("todos")?],
    )?;
    TauriSync::open(db, apps().sync_registry()?, transport).await
}

pub async fn list(sync: &TauriSync) -> ApiResult<Vec<Todo>> {
    let mut todos = sync.database().all::<Todo>().await?;
    todos.sort_by(|a, b| a.done.cmp(&b.done).then_with(|| b.id.cmp(&a.id)));
    Ok(todos)
}

fn title(value: &str) -> ApiResult<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 200 {
        return Err(ApiError::bad_request(
            "Название должно содержать от 1 до 200 символов",
        ));
    }
    Ok(value.into())
}

pub async fn create(sync: &TauriSync, value: &str) -> ApiResult<()> {
    sync.save(&Todo {
        id: Uuid::now_v7(),
        title: title(value)?,
        done: false,
    })
    .await
}

pub async fn update(
    sync: &TauriSync,
    id: Uuid,
    value: Option<String>,
    done: Option<bool>,
) -> ApiResult<()> {
    let mut todo = sync
        .database()
        .get::<Todo>(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Задача не найдена"))?;
    if let Some(value) = value {
        todo.title = title(&value)?;
    }
    if let Some(done) = done {
        todo.done = done;
    }
    sync.save(&todo).await
}

pub async fn remove(sync: &TauriSync, id: Uuid) -> ApiResult<()> {
    sync.delete::<Todo>(id).await
}
