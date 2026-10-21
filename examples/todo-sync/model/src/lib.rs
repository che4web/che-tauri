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

#[cfg(feature = "object-access")]
pub mod access {
    use super::*;
    use che_orm::ModelField;
    use che_rest::auth::{Group, User, access::*};
    #[derive(Debug, Model)]
    #[orm(table = "todo_user_grants", unique("todo_id", "user_id", "permission"))]
    pub struct UserGrant {
        #[orm(primary_key)]
        id: i64,
        #[orm(foreign_key=Todo,on_delete="cascade")]
        todo_id: Uuid,
        #[orm(foreign_key=User,on_delete="cascade")]
        user_id: i64,
        permission: String,
    }
    #[derive(Debug, Model)]
    #[orm(
        table = "todo_group_grants",
        unique("todo_id", "group_id", "permission")
    )]
    pub struct GroupGrant {
        #[orm(primary_key)]
        id: i64,
        #[orm(foreign_key=Todo,on_delete="cascade")]
        todo_id: Uuid,
        #[orm(foreign_key=Group,on_delete="cascade")]
        group_id: i64,
        permission: String,
    }
    impl ObjectGrant<Todo> for UserGrant {
        const OBJECT: ModelField<Self, Uuid> = Self::TODO_ID;
        const SUBJECT: ModelField<Self, i64> = Self::USER_ID;
        const PERMISSION: ModelField<Self, String> = Self::PERMISSION;
    }
    impl ObjectGrant<Todo> for GroupGrant {
        const OBJECT: ModelField<Self, Uuid> = Self::TODO_ID;
        const SUBJECT: ModelField<Self, i64> = Self::GROUP_ID;
        const PERMISSION: ModelField<Self, String> = Self::PERMISSION;
    }
    impl AccessModel for Todo {
        const ACCESS_KEY: &'static str = "todo.task";
        type UserGrant = UserGrant;
        type GroupGrant = GroupGrant;
        const CREATOR_PERMISSIONS: &'static [&'static str] = &[VIEW, CHANGE, DELETE, MANAGE_ACCESS];
    }
    pub fn schema() -> SchemaSet {
        Access::<Todo>::schema()
    }
}
