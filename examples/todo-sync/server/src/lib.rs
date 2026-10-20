use che_orm::{Database, DatabaseQuery, MigrationGraph, Model, SchemaSet};
use che_rest::{
    AppModule, AppResult, AppState, FilterSet, InstalledApps, IsAuthenticated, ModelSyncEndpoints,
    ModuleContext, SyncViewSet, ViewSet, ViewSetConfig,
};
use che_sync::Registry;
use todo_sync_model::{Todo, TodoSerializer, server_schema};

mod initial;

pub const DEMO_TOKEN: &str = "todo-demo-token";

#[derive(Clone)]
struct Todos;
impl ViewSet for Todos {
    type Model = Todo;
    type Serializer = TodoSerializer;
    type ListSerializer = TodoSerializer;
    type QuerySet = DatabaseQuery<Todo>;
    type FilterSet = FilterSet<Todo>;
    type Permission = IsAuthenticated;

    fn path(&self) -> &'static str {
        "/todos"
    }
    fn get_queryset(&self) -> Self::QuerySet {
        DatabaseQuery::new(Todo::query())
    }
    fn configure(&self, config: &mut ViewSetConfig<Self>) -> AppResult<()> {
        config.extend(ModelSyncEndpoints::new(self.clone()))
    }
}
impl SyncViewSet for Todos {
    fn sync_registry(&self) -> AppResult<Registry> {
        Ok(Registry::from_schema(&server_schema())?)
    }
}

struct TodoApp;
impl AppModule for TodoApp {
    fn name(&self) -> &'static str {
        "todo"
    }
    fn schema(&self) -> SchemaSet {
        server_schema()
    }
    fn sync_registry(&self) -> Option<Registry> {
        Some(Registry::from_schema(&server_schema()).expect("valid demo schema"))
    }
    fn migrations(&self) -> Vec<che_orm::Migration> {
        vec![initial::migration()]
    }
    fn init(&self, ctx: &mut ModuleContext) {
        ctx.viewset_with(Todos);
    }
}

pub fn apps() -> InstalledApps {
    InstalledApps::new()
        .add(che_rest::auth::module())
        .add(TodoApp)
}

/// Explicit setup command only; serving never creates application tables.
pub async fn migrate(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let mut migrations = che_rest::auth::migrations();
    migrations.push(initial::migration());
    db.migrate(MigrationGraph::new(migrations)?).await?;
    Ok(())
}

/// The demo has one account with whole-model access. No public signup/login.
pub async fn state(db: Database, token: &str) -> AppResult<AppState> {
    use che_rest::auth::{AuthToken, User, token_hash};
    let hash = token_hash(token);
    if db
        .query::<AuthToken>()
        .filter(AuthToken::KEY_HASH.eq(hash.clone()))
        .first(&db)
        .await?
        .is_none()
    {
        let user = match db
            .query::<User>()
            .filter(User::USERNAME.eq("todo-demo"))
            .first(&db)
            .await?
        {
            Some(user) => user,
            None => {
                db.create::<User>()
                    .set(User::USERNAME, "todo-demo")
                    .set(User::PASSWORD_HASH, "token-only-demo")
                    .execute()
                    .await?
            }
        };
        db.create::<AuthToken>()
            .set(AuthToken::USER_ID, user.id)
            .set(AuthToken::KEY_HASH, hash)
            .execute()
            .await?;
    }
    Ok(AppState::from_database(db))
}
