//! Explicit shared-object demo; existing protocol-2 apps() is unchanged.
use che_orm::{Database, DatabaseQuery, MigrationGraph, Model, SchemaSet, Uuid};
use che_rest::auth::{self, AuthToken, User, access::*};
use che_rest::{
    AppModule, AppResult, AppState, FilterSet, InstalledApps, ModuleContext, ObjectSyncEndpoints,
    SyncViewSet, ViewSet, ViewSetConfig,
};
use che_sync::Registry;
use todo_sync_model::{Todo, TodoSerializer};
mod migration;
#[derive(Clone)]
struct Todos;
impl ViewSet for Todos {
    type Model = Todo;
    type Serializer = TodoSerializer;
    type ListSerializer = TodoSerializer;
    type QuerySet = DatabaseQuery<Todo>;
    type FilterSet = FilterSet<Todo>;
    type Permission = Access<Todo>;
    fn path(&self) -> &'static str {
        "/todos"
    }
    fn get_queryset(&self) -> Self::QuerySet {
        DatabaseQuery::new(Todo::query())
    }
    fn configure(&self, c: &mut ViewSetConfig<Self>) -> AppResult<()> {
        c.extend(ObjectSyncEndpoints::new(self.clone()))
    }
}
impl SyncViewSet for Todos {
    fn sync_registry(&self) -> AppResult<Registry> {
        Ok(Registry::from_schema(&todo_sync_model::server_schema())?)
    }
}
struct SharedApp;
impl AppModule for SharedApp {
    fn name(&self) -> &'static str {
        "todo"
    }
    fn schema(&self) -> SchemaSet {
        todo_sync_model::server_schema().merge(todo_sync_model::access::schema())
    }
    fn sync_registry(&self) -> Option<Registry> {
        Some(Registry::from_schema(&todo_sync_model::server_schema()).expect("todo registry"))
    }
    fn migrations(&self) -> Vec<che_orm::Migration> {
        vec![super::initial::migration(), migration::migration()]
    }
    fn init(&self, c: &mut ModuleContext) {
        c.viewset_with(Todos);
    }
}
pub fn apps() -> InstalledApps {
    InstalledApps::new()
        .add(auth::module())
        .add(che_rest::sync::object_schema_module())
        .add(SharedApp)
}
pub async fn migrate(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let mut history = auth::migrations();
    history.extend([super::initial::migration(), migration::migration()]);
    db.migrate(MigrationGraph::new(history)?).await?;
    Ok(())
}
/// Demo credentials only: alice / todo-alice and bob / todo-bob. Tables must exist.
pub async fn state(db: Database) -> AppResult<AppState> {
    for name in ["alice", "bob"] {
        let user = match db
            .fetch_one(User::query().filter(User::USERNAME.eq(name)))
            .await?
        {
            Some(user) => user,
            None => {
                db.create::<User>()
                    .set(User::USERNAME, name)
                    .set(User::PASSWORD_HASH, "unused-token-only-demo")
                    .execute()
                    .await?
            }
        };
        let hash = auth::token_hash(&format!("todo-{name}"));
        if db
            .fetch_one(AuthToken::query().filter(AuthToken::KEY_HASH.eq(hash.clone())))
            .await?
            .is_none()
        {
            db.create::<AuthToken>()
                .set(AuthToken::USER_ID, user.id)
                .set(AuthToken::KEY_HASH, hash)
                .execute()
                .await?;
        }
        db.transaction_result(move |tx| {
            Box::pin(
                async move { Access::<Todo>::grant_model(tx, Subject::User(user.id), ADD).await },
            )
        })
        .await?;
    }
    Ok(AppState::from_database(db))
}
/// Trusted management command, never a public grant endpoint.
pub async fn share(db: &Database, id: Uuid, username: &str, allow: bool) -> AppResult<()> {
    let user = db
        .fetch_one(User::query().filter(User::USERNAME.eq(username)))
        .await?
        .ok_or(che_rest::AppError::NotFound)?;
    db.transaction_result(move |tx| {
        Box::pin(async move {
            for p in [VIEW, CHANGE, DELETE] {
                if allow {
                    Access::<Todo>::grant(tx, Subject::User(user.id), p, id).await?;
                } else {
                    Access::<Todo>::revoke(tx, Subject::User(user.id), p, id).await?;
                }
            }
            Ok::<_, che_rest::AppError>(())
        })
    })
    .await
}
