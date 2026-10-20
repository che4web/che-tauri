use che_orm::{ColumnType, Model, SchemaSet, SqlCompiler, SqliteDialect, TableSchema};
use serde::Serialize;

use crate::{FilterSet, ModelSerializer, api::ResourceRegistration};

pub trait AppModule {
    fn name(&self) -> &'static str;
    fn init(&self, ctx: &mut ModuleContext);

    fn migrations(&self) -> Vec<che_orm::Migration> {
        Vec::new()
    }
}

#[derive(Default)]
pub struct InstalledApps {
    modules: Vec<Box<dyn AppModule>>,
}

impl InstalledApps {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add<M>(mut self, module: M) -> Self
    where
        M: AppModule + 'static,
    {
        self.modules.push(Box::new(module));
        self
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn AppModule> {
        self.modules.iter().map(Box::as_ref)
    }

    pub fn find(&self, name: &str) -> Option<&dyn AppModule> {
        self.iter().find(|module| module.name() == name)
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.iter().map(AppModule::name)
    }

    /// Build a protocol-2 client registry in module registration order.
    /// Call after applying the modules' compiled migrations.
    #[cfg(feature = "sync")]
    pub fn sync_registry(&self) -> crate::ApiResult<che_sync::Registry> {
        let mut schema = SchemaSet::new();
        for module in self.iter() {
            let mut ctx = ModuleContext::new();
            module.init(&mut ctx);
            schema = schema.merge(ctx.schemas);
        }
        Ok(che_sync::Registry::from_schema(&schema)?)
    }
}

#[derive(Default)]
pub struct ModuleContext {
    sql: Vec<String>,
    schemas: SchemaSet,
    api_endpoints: Vec<ApiEndpoint>,
    resources: Vec<ResourceRegistration>,
    cache_tables: Vec<&'static str>,
}

#[derive(Debug, Clone)]
pub struct ApiEndpoint {
    pub model_name: String,
    pub resource: String,
    pub fields: Vec<ApiField>,
    pub filters: Vec<ApiFilter>,
}

#[derive(Debug, Clone)]
pub struct ApiField {
    pub name: String,
    pub source: String,
    pub ty: ColumnType,
    pub related_model: Option<String>,
    pub ts_type: Option<String>,
    pub input_ts_type: Option<String>,
    pub read_only: bool,
    pub write_only: bool,
    pub required: bool,
    pub nullable: bool,
    pub has_default: bool,
}

#[derive(Debug, Clone)]
pub struct ApiFilter {
    pub name: String,
    pub source: String,
    pub ty: ColumnType,
    pub nullable: bool,
}

impl ModuleContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a UUID sync model and its client service schema for compiled
    /// migrations. This does not create tables or register i64 CRUD handlers.
    #[cfg(feature = "sync")]
    pub fn sync_model<M: che_sync::SyncModel>(&mut self) {
        self.schemas = std::mem::take(&mut self.schemas).client_model::<M>();
    }

    pub fn model<M>(&mut self)
    where
        M: Model,
    {
        let compiled = SqlCompiler::<SqliteDialect>::compile_schema(&M::schema());
        self.sql.push(
            compiled
                .table
                .replacen("CREATE TABLE ", "CREATE TABLE IF NOT EXISTS ", 1),
        );
        self.sql.extend(
            compiled
                .indexes
                .into_iter()
                .map(|sql| sql.replacen("CREATE INDEX ", "CREATE INDEX IF NOT EXISTS ", 1)),
        );
        self.schemas = std::mem::take(&mut self.schemas).model::<M>();
    }

    pub fn create_table<M>(&mut self)
    where
        M: Model,
    {
        self.model::<M>();
    }

    pub fn resource<M>(
        &mut self,
        resource: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) where
        M: Model + Serialize + Send + Sync + 'static,
    {
        self.model::<M>();
        let registration = ResourceRegistration::from_model::<M>(resource, serializer, filterset);
        self.api_endpoints.push(ApiEndpoint {
            model_name: rust_type_name::<M>(),
            resource: resource.to_string(),
            fields: registration.fields.clone(),
            filters: registration.filters.clone(),
        });
        self.resources.push(registration);
    }

    pub fn remote_resource<M>(
        &mut self,
        resource: &'static str,
        remote_path: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) where
        M: Model + Serialize + Send + Sync + 'static,
    {
        self.mapped_remote_resource(resource, remote_path, serializer, filterset);
    }

    pub fn raw_remote_resource<M>(
        &mut self,
        resource: &'static str,
        remote_path: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) where
        M: Model + Serialize + Send + Sync + 'static,
    {
        let registration = ResourceRegistration::from_remote_model::<M>(
            resource,
            remote_path,
            serializer,
            filterset,
        );
        self.api_endpoints.push(ApiEndpoint {
            model_name: rust_type_name::<M>(),
            resource: resource.to_string(),
            fields: registration.fields.clone(),
            filters: registration.filters.clone(),
        });
        self.resources.push(registration);
    }

    pub fn mapped_remote_resource<M>(
        &mut self,
        resource: &'static str,
        remote_path: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) where
        M: Model + Serialize + Send + Sync + 'static,
    {
        let registration = ResourceRegistration::from_mapped_remote_model::<M>(
            resource,
            remote_path,
            serializer,
            filterset,
        );
        self.api_endpoints.push(ApiEndpoint {
            model_name: rust_type_name::<M>(),
            resource: resource.to_string(),
            fields: registration.fields.clone(),
            filters: registration.filters.clone(),
        });
        self.resources.push(registration);
    }

    pub fn cached_mapped_remote_resource<M>(
        &mut self,
        resource: &'static str,
        remote_path: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) where
        M: Model + Serialize + Send + Sync + 'static,
    {
        let registration = ResourceRegistration::from_cached_mapped_remote_model::<M>(
            resource,
            remote_path,
            serializer,
            filterset,
        );
        self.model::<M>();
        self.cache_tables.push(M::table_name());
        self.api_endpoints.push(ApiEndpoint {
            model_name: rust_type_name::<M>(),
            resource: resource.to_string(),
            fields: registration.fields.clone(),
            filters: registration.filters.clone(),
        });
        self.resources.push(registration);
    }

    pub fn model_schemas(&self) -> &[TableSchema] {
        self.schemas.tables()
    }

    pub fn schema(&self) -> &SchemaSet {
        &self.schemas
    }

    pub fn api_endpoints(&self) -> &[ApiEndpoint] {
        &self.api_endpoints
    }

    pub(crate) fn into_parts(self) -> (Vec<String>, Vec<ResourceRegistration>, Vec<&'static str>) {
        (self.sql, self.resources, self.cache_tables)
    }
}

fn rust_type_name<M>() -> String {
    std::any::type_name::<M>()
        .rsplit("::")
        .next()
        .unwrap_or("Model")
        .to_string()
}
