use std::sync::Arc;

use che_orm::{ColumnType, MigrationError, Model, ProjectState, SchemaSet};
use serde::Serialize;

use crate::{ApiError, ApiResult, AppState, FilterSet, ModelSerializer, api::ResourceRegistration};

pub trait AppModule: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    /// Database schema only; used without invoking runtime registration.
    fn schema(&self) -> SchemaSet;
    /// Declarative invoke registration; may also run during API generation.
    fn init(&self, ctx: &mut ModuleContext);

    fn migrations(&self) -> Vec<che_orm::Migration> {
        Vec::new()
    }

    fn subscribe(&self, _state: &AppState) {}

    fn start(&self, _state: &AppState) {}
}

#[derive(Default, Clone)]
pub struct InstalledApps {
    modules: Vec<Arc<dyn AppModule>>,
}

impl InstalledApps {
    pub fn new() -> Self {
        Self::default()
    }

    #[expect(
        clippy::should_implement_trait,
        reason = "Builder-style module registration, not arithmetic addition"
    )]
    pub fn add<M>(mut self, module: M) -> Self
    where
        M: AppModule + 'static,
    {
        self.modules.push(Arc::new(module));
        self
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn AppModule> {
        self.modules.iter().map(Arc::as_ref)
    }

    pub fn find(&self, name: &str) -> Option<&dyn AppModule> {
        self.iter().find(|module| module.name() == name)
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.iter().map(AppModule::name)
    }

    /// Collects schema metadata without registering resources or starting modules.
    pub fn schema(&self) -> SchemaSet {
        self.iter().fold(SchemaSet::new(), |schema, module| {
            schema.merge(module.schema())
        })
    }

    pub fn migration_state(&self) -> Result<ProjectState, MigrationError> {
        let schemas: Vec<_> = self
            .iter()
            .map(|module| (module.name(), module.schema()))
            .collect();
        ProjectState::from_app_schemas(schemas.iter().map(|(name, schema)| (*name, schema)))
    }

    /// Build a client registry from the declared schemas, without calling init.
    #[cfg(feature = "sync")]
    pub fn sync_registry(&self) -> ApiResult<che_sync::Registry> {
        let schema = self.schema();
        if schema
            .sync_models()
            .iter()
            .any(|model| model.role != che_orm::ModelRole::Client)
        {
            return Err(ApiError::bad_request(
                "Tauri sync models must use the client database role",
            ));
        }
        Ok(che_sync::Registry::from_schema(&schema)?)
    }
}

#[derive(Default)]
pub struct ModuleContext {
    local_tables: Vec<&'static str>,
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

    pub fn resource<M>(
        &mut self,
        resource: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) where
        M: Model + Serialize + Send + Sync + 'static,
    {
        self.local_tables.push(M::table_name());
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
        self.local_tables.push(M::table_name());
        self.cache_tables.push(M::table_name());
        self.api_endpoints.push(ApiEndpoint {
            model_name: rust_type_name::<M>(),
            resource: resource.to_string(),
            fields: registration.fields.clone(),
            filters: registration.filters.clone(),
        });
        self.resources.push(registration);
    }

    pub fn api_endpoints(&self) -> &[ApiEndpoint] {
        &self.api_endpoints
    }

    pub(crate) fn validate(&self, schema: &SchemaSet) -> ApiResult<()> {
        schema
            .validate()
            .map_err(|error| ApiError::bad_request(format!("invalid app schema: {error:?}")))?;
        let mut resources = std::collections::HashSet::new();
        for resource in &self.resources {
            if !resources.insert(resource.resource) {
                return Err(ApiError::bad_request(format!(
                    "duplicate resource: {}",
                    resource.resource
                )));
            }
        }
        for table in &self.local_tables {
            if table.starts_with("_sync_")
                || schema
                    .sync_models()
                    .iter()
                    .any(|model| model.schema.name == *table)
            {
                return Err(ApiError::bad_request(format!(
                    "sync table {table} cannot be used by ordinary local or cached resources"
                )));
            }
            if !schema.tables().iter().any(|model| model.name == *table) {
                return Err(ApiError::bad_request(format!(
                    "local table {table} is missing from AppModule::schema()"
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn into_parts(self) -> (Vec<ResourceRegistration>, Vec<&'static str>) {
        (self.resources, self.cache_tables)
    }
}

fn rust_type_name<M>() -> String {
    std::any::type_name::<M>()
        .rsplit("::")
        .next()
        .unwrap_or("Model")
        .to_string()
}
