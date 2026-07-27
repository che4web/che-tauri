use che_orm::{FieldType, Model, ModelSchema, SqliteModel, create_table_sql};

use crate::{FilterSet, ModelSerializer, api::ResourceRegistration};

pub trait AppModule {
    fn name(&self) -> &'static str;
    fn init(&self, ctx: &mut ModuleContext);
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
}

#[derive(Default)]
pub struct ModuleContext {
    sql: Vec<String>,
    schemas: Vec<ModelSchema>,
    api_endpoints: Vec<ApiEndpoint>,
    resources: Vec<ResourceRegistration>,
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
    pub ty: FieldType,
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
    pub ty: FieldType,
    pub nullable: bool,
}

impl ModuleContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn model<M>(&mut self)
    where
        M: Model,
    {
        self.sql.push(create_table_sql::<M>());
        self.schemas.push(ModelSchema::from_model::<M>());
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
        M: SqliteModel<Id = i64>,
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
        M: SqliteModel<Id = i64>,
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
        M: SqliteModel<Id = i64>,
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
        M: SqliteModel<Id = i64>,
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
        M: SqliteModel<Id = i64>,
    {
        let registration = ResourceRegistration::from_cached_mapped_remote_model::<M>(
            resource,
            remote_path,
            serializer,
            filterset,
        );
        self.model::<M>();
        self.api_endpoints.push(ApiEndpoint {
            model_name: rust_type_name::<M>(),
            resource: resource.to_string(),
            fields: registration.fields.clone(),
            filters: registration.filters.clone(),
        });
        self.resources.push(registration);
    }

    pub fn model_schemas(&self) -> &[ModelSchema] {
        &self.schemas
    }

    pub fn api_endpoints(&self) -> &[ApiEndpoint] {
        &self.api_endpoints
    }

    pub(crate) fn into_parts(self) -> (Vec<String>, Vec<ResourceRegistration>) {
        (self.sql, self.resources)
    }
}

fn rust_type_name<M>() -> String {
    std::any::type_name::<M>()
        .rsplit("::")
        .next()
        .unwrap_or("Model")
        .to_string()
}
