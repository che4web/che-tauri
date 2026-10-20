use che_orm::{Model, ModelSerializer, SchemaSet, Uuid};

#[derive(Debug, Clone, Model, serde::Serialize)]
#[orm(table = "todos", sync, sync_name = "todo.task")]
pub struct Todo {
    #[orm(primary_key)]
    pub id: Uuid,
    pub title: String,
    pub done: bool,
}

#[derive(ModelSerializer)]
#[serializer(model = Todo)]
pub struct TodoSerializer {
    #[serializer(read_only)]
    pub id: Uuid,
    pub title: String,
    pub done: bool,
}

pub fn client_schema() -> SchemaSet {
    SchemaSet::new().client_model::<Todo>()
}

pub fn server_schema() -> SchemaSet {
    SchemaSet::new().server_model::<Todo>()
}
