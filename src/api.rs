use std::{collections::HashMap, marker::PhantomData, time::Duration};

use async_trait::async_trait;
use che_orm::{
    Assignment, ColumnRef, InsertAst, InsertValue, Model, QueryAst, TableRef, UpdateAst,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppState, Field, Filter, FilterSet, ModelSerializer,
    error::{ApiError, ApiResult},
    module::InstalledApps,
    serializer::upsert_normalized_model,
};

#[derive(Debug, Deserialize)]
pub struct ApiRequest {
    pub resource: String,
    pub action: ApiAction,
    pub id: Option<i64>,
    #[serde(default)]
    pub params: HashMap<String, String>,
    pub payload: Option<Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiAction {
    List,
    Retrieve,
    Create,
    Update,
    Delete,
}

#[derive(Debug, Serialize)]
pub struct AuthLoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AuthTokenResponse {
    pub token: String,
}

#[async_trait]
pub trait InvokeResource: Send + Sync {
    fn resource(&self) -> &'static str;

    async fn list(&self, state: &AppState, params: HashMap<String, String>) -> ApiResult<Value>;
    async fn retrieve(&self, state: &AppState, id: i64) -> ApiResult<Value>;
    async fn create(&self, state: &AppState, payload: Value) -> ApiResult<Value>;
    async fn update(&self, state: &AppState, id: i64, payload: Value) -> ApiResult<Value>;
    async fn delete(&self, state: &AppState, id: i64) -> ApiResult<Value>;
}

pub struct ModelInvokeSet<M> {
    resource: &'static str,
    serializer: ModelSerializer<M>,
    filterset: FilterSet<M>,
    _model: PhantomData<M>,
}

pub struct RawRemoteInvokeSet<M> {
    resource: &'static str,
    remote_path: &'static str,
    _model: PhantomData<M>,
}

pub struct MappedRemoteInvokeSet<M> {
    resource: &'static str,
    remote_path: &'static str,
    serializer: ModelSerializer<M>,
    filterset: FilterSet<M>,
    _model: PhantomData<M>,
}

pub struct CachedMappedRemoteInvokeSet<M> {
    resource: &'static str,
    remote_path: &'static str,
    serializer: ModelSerializer<M>,
    filterset: FilterSet<M>,
    _model: PhantomData<M>,
}

impl<M> ModelInvokeSet<M>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    pub fn new(
        resource: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) -> Self {
        Self {
            resource,
            serializer,
            filterset,
            _model: PhantomData,
        }
    }
}

impl<M> RawRemoteInvokeSet<M>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    pub fn new(resource: &'static str, remote_path: &'static str) -> Self {
        Self {
            resource,
            remote_path,
            _model: PhantomData,
        }
    }
}

impl<M> MappedRemoteInvokeSet<M>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    pub fn new(
        resource: &'static str,
        remote_path: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) -> Self {
        Self {
            resource,
            remote_path,
            serializer,
            filterset,
            _model: PhantomData,
        }
    }
}

impl<M> CachedMappedRemoteInvokeSet<M>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    pub fn new(
        resource: &'static str,
        remote_path: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) -> Self {
        Self {
            resource,
            remote_path,
            serializer,
            filterset,
            _model: PhantomData,
        }
    }
}

#[async_trait]
impl<M> InvokeResource for ModelInvokeSet<M>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    fn resource(&self) -> &'static str {
        self.resource
    }

    async fn list(&self, state: &AppState, params: HashMap<String, String>) -> ApiResult<Value> {
        local_list(state, &self.serializer, &self.filterset, &params).await
    }

    async fn retrieve(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        let model = state
            .db()
            .get::<M>(crate::database::primary_key::<M>(id)?)
            .await?
            .ok_or_else(|| ApiError::not_found("model not found"))?;
        Ok(self.serializer.to_json_async(state.db(), &model).await?)
    }

    async fn create(&self, state: &AppState, payload: Value) -> ApiResult<Value> {
        let values = self
            .serializer
            .create_values(payload)?
            .into_iter()
            .map(|(field, value)| InsertValue {
                column: ColumnRef::new(M::table_name(), field),
                value,
            })
            .collect();
        let model = crate::database::write_model::<M>(
            state.db(),
            QueryAst::Insert(InsertAst {
                table: TableRef::new(M::table_name()),
                values,
                returning: M::columns()
                    .iter()
                    .map(|name| ColumnRef::new(M::table_name(), name))
                    .collect(),
            }),
        )
        .await?
        .ok_or_else(|| ApiError::internal("insert did not return a row"))?;
        Ok(self.serializer.to_json_async(state.db(), &model).await?)
    }

    async fn update(&self, state: &AppState, id: i64, payload: Value) -> ApiResult<Value> {
        let mut assignments = self
            .serializer
            .update_values(payload)?
            .into_iter()
            .map(|(field, value)| Assignment {
                column: ColumnRef::new(M::table_name(), field),
                value,
            })
            .collect::<Vec<_>>();
        if assignments.is_empty() {
            return Err(ApiError::bad_request("empty update"));
        }
        assignments.extend(M::managed_update_values());
        let model = crate::database::write_model::<M>(
            state.db(),
            QueryAst::Update(UpdateAst {
                table: TableRef::new(M::table_name()),
                assignments,
                filter: Some(M::primary_key().eq(crate::database::primary_key::<M>(id)?)),
                returning: M::columns()
                    .iter()
                    .map(|name| ColumnRef::new(M::table_name(), name))
                    .collect(),
                allow_all: false,
            }),
        )
        .await?
        .ok_or_else(|| ApiError::not_found("model not found"))?;
        Ok(self.serializer.to_json_async(state.db(), &model).await?)
    }

    async fn delete(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        if !state
            .db()
            .delete::<M>(crate::database::primary_key::<M>(id)?)
            .await?
        {
            return Err(ApiError::not_found("model not found"));
        }
        Ok(json!({ "deleted": true }))
    }
}

#[async_trait]
impl<M> InvokeResource for RawRemoteInvokeSet<M>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    fn resource(&self) -> &'static str {
        self.resource
    }

    async fn list(&self, state: &AppState, params: HashMap<String, String>) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().get(remote_url(state, self.remote_path)?),
        )?
        .query(&params)
        .send()
        .await?;
        remote_json(response).await
    }

    async fn retrieve(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().get(format!(
                "{}/{}/",
                remote_url(state, self.remote_path)?.trim_end_matches('/'),
                id
            )),
        )?
        .send()
        .await?;
        remote_json(response).await
    }

    async fn create(&self, state: &AppState, payload: Value) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().post(remote_url(state, self.remote_path)?),
        )?
        .json(&payload)
        .send()
        .await?;
        remote_json(response).await
    }

    async fn update(&self, state: &AppState, id: i64, payload: Value) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().patch(format!(
                "{}/{}/",
                remote_url(state, self.remote_path)?.trim_end_matches('/'),
                id
            )),
        )?
        .json(&payload)
        .send()
        .await?;
        remote_json(response).await
    }

    async fn delete(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().delete(format!(
                "{}/{}/",
                remote_url(state, self.remote_path)?.trim_end_matches('/'),
                id
            )),
        )?
        .send()
        .await?;
        if response.status().is_success() {
            Ok(json!({ "deleted": true }))
        } else {
            remote_error(response).await
        }
    }
}

#[async_trait]
impl<M> InvokeResource for MappedRemoteInvokeSet<M>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    fn resource(&self) -> &'static str {
        self.resource
    }

    async fn list(&self, state: &AppState, params: HashMap<String, String>) -> ApiResult<Value> {
        self.filterset.validate_json_params(&params)?;
        let original_params = params.clone();
        let params = self.filterset.remote_params(&params);
        let response = with_token(
            state,
            remote_client().get(remote_url(state, self.remote_path)?),
        )?
        .query(&params)
        .send()
        .await?;
        let value = remote_json(response).await?;
        let value = normalize_remote_list(value, self.serializer);
        Ok(self.filterset.apply_json(value, &original_params))
    }

    async fn retrieve(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().get(format!(
                "{}/{}/",
                remote_url(state, self.remote_path)?.trim_end_matches('/'),
                id
            )),
        )?
        .send()
        .await?;
        let value = remote_json(response).await?;
        Ok(self.serializer.normalize_remote_value(value))
    }

    async fn create(&self, state: &AppState, payload: Value) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().post(remote_url(state, self.remote_path)?),
        )?
        .json(&payload)
        .send()
        .await?;
        let value = remote_json(response).await?;
        Ok(self.serializer.normalize_remote_value(value))
    }

    async fn update(&self, state: &AppState, id: i64, payload: Value) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().patch(format!(
                "{}/{}/",
                remote_url(state, self.remote_path)?.trim_end_matches('/'),
                id
            )),
        )?
        .json(&payload)
        .send()
        .await?;
        let value = remote_json(response).await?;
        Ok(self.serializer.normalize_remote_value(value))
    }

    async fn delete(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().delete(format!(
                "{}/{}/",
                remote_url(state, self.remote_path)?.trim_end_matches('/'),
                id
            )),
        )?
        .send()
        .await?;
        if response.status().is_success() {
            Ok(json!({ "deleted": true }))
        } else {
            remote_error(response).await
        }
    }
}

#[async_trait]
impl<M> InvokeResource for CachedMappedRemoteInvokeSet<M>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    fn resource(&self) -> &'static str {
        self.resource
    }

    async fn list(&self, state: &AppState, params: HashMap<String, String>) -> ApiResult<Value> {
        let session = state.cache_session(M::table_name()).await?;
        session.finish(state, self.cached_list(state, params).await)
    }

    async fn retrieve(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        let session = state.cache_session(M::table_name()).await?;
        session.finish(state, self.cached_retrieve(state, id).await)
    }

    async fn create(&self, state: &AppState, payload: Value) -> ApiResult<Value> {
        let session = state.cache_session(M::table_name()).await?;
        session.finish(state, self.cached_create(state, payload).await)
    }

    async fn update(&self, state: &AppState, id: i64, payload: Value) -> ApiResult<Value> {
        let session = state.cache_session(M::table_name()).await?;
        session.finish(state, self.cached_update(state, id, payload).await)
    }

    async fn delete(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        let session = state.cache_session(M::table_name()).await?;
        session.finish(state, self.cached_delete(state, id).await)
    }
}

impl<M> CachedMappedRemoteInvokeSet<M>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    async fn cached_list(
        &self,
        state: &AppState,
        params: HashMap<String, String>,
    ) -> ApiResult<Value> {
        self.filterset.validate_json_params(&params)?;
        let force_remote = params
            .get("force_remote")
            .is_some_and(|value| value == "true");
        let mut local_params = params.clone();
        local_params.remove("force_remote");

        if !force_remote
            && let Ok(local_value) =
                local_list::<M>(state, &self.serializer, &self.filterset, &local_params).await
            && local_value
                .get("results")
                .and_then(Value::as_array)
                .is_some_and(|results| !results.is_empty())
        {
            return Ok(local_value);
        }

        let remote_params = self.filterset.remote_params(&local_params);
        let response = with_token(
            state,
            remote_client().get(remote_url(state, self.remote_path)?),
        )?
        .query(&remote_params)
        .send()
        .await?;
        let value = remote_json(response).await?;
        let value = normalize_remote_list(value, self.serializer);
        cache_normalized_list::<M>(state, self.serializer, &value).await?;
        Ok(self.filterset.apply_json(value, &local_params))
    }

    async fn cached_retrieve(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        if let Ok(Some(value)) = local_retrieve::<M>(state, &self.serializer, id).await {
            return Ok(value);
        }

        let response = with_token(
            state,
            remote_client().get(format!(
                "{}/{}/",
                remote_url(state, self.remote_path)?.trim_end_matches('/'),
                id
            )),
        )?
        .send()
        .await?;
        let value = remote_json(response).await?;
        let value = self.serializer.normalize_remote_value(value);
        upsert_normalized_model(state.db(), self.serializer, value.clone()).await?;
        Ok(value)
    }

    async fn cached_create(&self, state: &AppState, payload: Value) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().post(remote_url(state, self.remote_path)?),
        )?
        .json(&payload)
        .send()
        .await?;
        let value = remote_json(response).await?;
        let value = self.serializer.normalize_remote_value(value);
        upsert_normalized_model(state.db(), self.serializer, value.clone()).await?;
        Ok(value)
    }

    async fn cached_update(&self, state: &AppState, id: i64, payload: Value) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().patch(format!(
                "{}/{}/",
                remote_url(state, self.remote_path)?.trim_end_matches('/'),
                id
            )),
        )?
        .json(&payload)
        .send()
        .await?;
        let value = remote_json(response).await?;
        let value = self.serializer.normalize_remote_value(value);
        upsert_normalized_model(state.db(), self.serializer, value.clone()).await?;
        Ok(value)
    }

    async fn cached_delete(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        let response = with_token(
            state,
            remote_client().delete(format!(
                "{}/{}/",
                remote_url(state, self.remote_path)?.trim_end_matches('/'),
                id
            )),
        )?
        .send()
        .await?;

        if response.status().is_success() {
            state
                .db()
                .delete::<M>(crate::database::primary_key::<M>(id)?)
                .await?;
            Ok(json!({ "deleted": true }))
        } else {
            remote_error(response).await
        }
    }
}

pub struct RegisteredResource {
    inner: Box<dyn InvokeResource>,
}

impl std::fmt::Debug for RegisteredResource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegisteredResource")
            .field("resource", &self.resource())
            .finish_non_exhaustive()
    }
}

impl RegisteredResource {
    pub fn new(resource: Box<dyn InvokeResource>) -> Self {
        Self { inner: resource }
    }

    pub fn resource(&self) -> &'static str {
        self.inner.resource()
    }

    pub async fn dispatch(&self, state: &AppState, request: ApiRequest) -> ApiResult<Value> {
        match request.action {
            ApiAction::List => self.inner.list(state, request.params).await,
            ApiAction::Retrieve => {
                self.inner
                    .retrieve(state, required_id(request.id, request.action)?)
                    .await
            }
            ApiAction::Create => {
                self.inner
                    .create(state, required_payload(request.payload, request.action)?)
                    .await
            }
            ApiAction::Update => {
                self.inner
                    .update(
                        state,
                        required_id(request.id, request.action)?,
                        required_payload(request.payload, request.action)?,
                    )
                    .await
            }
            ApiAction::Delete => {
                self.inner
                    .delete(state, required_id(request.id, request.action)?)
                    .await
            }
        }
    }
}

pub struct ResourceRegistration {
    pub resource: &'static str,
    pub fields: Vec<crate::ApiField>,
    pub filters: Vec<crate::ApiFilter>,
    pub invoke_resource: RegisteredResource,
}

impl ResourceRegistration {
    pub fn from_model<M>(
        resource: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) -> Self
    where
        M: Model + Serialize + Send + Sync + 'static,
    {
        Self {
            resource,
            fields: api_fields::<M>(serializer.fields()),
            filters: api_filters::<M>(filterset.filters()),
            invoke_resource: RegisteredResource::new(Box::new(ModelInvokeSet::<M>::new(
                resource, serializer, filterset,
            ))),
        }
    }

    pub fn from_remote_model<M>(
        resource: &'static str,
        remote_path: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) -> Self
    where
        M: Model + Serialize + Send + Sync + 'static,
    {
        Self {
            resource,
            fields: api_fields::<M>(serializer.fields()),
            filters: api_filters::<M>(filterset.filters()),
            invoke_resource: RegisteredResource::new(Box::new(RawRemoteInvokeSet::<M>::new(
                resource,
                remote_path,
            ))),
        }
    }

    pub fn from_mapped_remote_model<M>(
        resource: &'static str,
        remote_path: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) -> Self
    where
        M: Model + Serialize + Send + Sync + 'static,
    {
        Self {
            resource,
            fields: api_fields::<M>(serializer.fields()),
            filters: api_filters::<M>(filterset.filters()),
            invoke_resource: RegisteredResource::new(Box::new(MappedRemoteInvokeSet::<M>::new(
                resource,
                remote_path,
                serializer,
                filterset,
            ))),
        }
    }

    pub fn from_cached_mapped_remote_model<M>(
        resource: &'static str,
        remote_path: &'static str,
        serializer: ModelSerializer<M>,
        filterset: FilterSet<M>,
    ) -> Self
    where
        M: Model + Serialize + Send + Sync + 'static,
    {
        Self {
            resource,
            fields: api_fields::<M>(serializer.fields()),
            filters: api_filters::<M>(filterset.filters()),
            invoke_resource: RegisteredResource::new(Box::new(
                CachedMappedRemoteInvokeSet::<M>::new(resource, remote_path, serializer, filterset),
            )),
        }
    }
}

async fn local_list<M>(
    state: &AppState,
    serializer: &ModelSerializer<M>,
    filterset: &FilterSet<M>,
    params: &HashMap<String, String>,
) -> ApiResult<Value>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    let count_params = params
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "limit" | "offset" | "ordering"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<HashMap<_, _>>();
    let count = state
        .db()
        .count_query(filterset.apply(M::query(), &count_params)?)
        .await?;
    let query = filterset.apply(M::query(), params)?;
    let models = state.db().fetch_all(query).await?;
    let mut results = Vec::with_capacity(models.len());
    for model in &models {
        results.push(serializer.to_json_async(state.db(), model).await?);
    }

    Ok(json!({
        "count": count,
        "results": results,
    }))
}

async fn local_retrieve<M>(
    state: &AppState,
    serializer: &ModelSerializer<M>,
    id: i64,
) -> ApiResult<Option<Value>>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    match state
        .db()
        .get::<M>(crate::database::primary_key::<M>(id)?)
        .await?
    {
        Some(model) => Ok(Some(serializer.to_json_async(state.db(), &model).await?)),
        None => Ok(None),
    }
}

async fn cache_normalized_list<M>(
    state: &AppState,
    serializer: ModelSerializer<M>,
    value: &Value,
) -> ApiResult<()>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    let Some(results) = value.get("results").and_then(Value::as_array) else {
        return Ok(());
    };

    for item in results {
        upsert_normalized_model(state.db(), serializer, item.clone()).await?;
    }

    Ok(())
}

fn normalize_remote_list<M: che_orm::Model>(value: Value, serializer: ModelSerializer<M>) -> Value {
    if let Some(object) = value.as_object()
        && let Some(results) = object.get("results").and_then(Value::as_array)
    {
        let normalized = results
            .iter()
            .cloned()
            .map(|item| serializer.normalize_remote_value(item))
            .collect::<Vec<_>>();
        let count = object
            .get("count")
            .cloned()
            .unwrap_or_else(|| Value::from(normalized.len()));

        return json!({
            "count": count,
            "results": normalized,
        });
    }

    if let Some(results) = value.as_array() {
        let normalized = results
            .iter()
            .cloned()
            .map(|item| serializer.normalize_remote_value(item))
            .collect::<Vec<_>>();
        return json!({
            "count": normalized.len(),
            "results": normalized,
        });
    }

    value
}

pub struct TauriApi {
    state: AppState,
    resources: HashMap<String, RegisteredResource>,
    apps: InstalledApps,
}

impl std::fmt::Debug for TauriApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TauriApi")
            .field("state", &self.state)
            .field("resources", &self.resources)
            .finish_non_exhaustive()
    }
}

impl TauriApi {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            resources: HashMap::new(),
            apps: InstalledApps::new(),
        }
    }

    /// Store modules; schema/resource registration happens only in build.
    pub fn install(mut self, apps: InstalledApps) -> Self {
        self.apps = apps;
        self
    }

    /// Assemble resources and start modules. Database migrations are explicit.
    pub async fn build(mut self) -> ApiResult<Self> {
        let mut schema = che_orm::SchemaSet::new();
        let mut ctx = crate::ModuleContext::new();
        for module in self.apps.iter() {
            schema = schema.merge(module.schema());
            module.init(&mut ctx);
        }
        ctx.validate(&schema)?;
        let (resources, cache_tables) = ctx.into_parts();
        for table in cache_tables {
            self.state.register_cache_table(table);
        }
        self.resources = resources
            .into_iter()
            .map(|resource| (resource.resource.to_string(), resource.invoke_resource))
            .collect();
        for module in self.apps.iter() {
            module.subscribe(&self.state);
        }
        for module in self.apps.iter() {
            module.start(&self.state);
        }
        Ok(self)
    }

    pub async fn dispatch(&self, request: ApiRequest) -> ApiResult<Value> {
        let resource = self.resources.get(&request.resource).ok_or_else(|| {
            ApiError::not_found(format!("unknown resource: {}", request.resource))
        })?;
        resource.dispatch(&self.state, request).await
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub async fn login(&self, username: String, password: String) -> ApiResult<AuthTokenResponse> {
        let response = remote_client()
            .post(remote_auth_url(&self.state)?)
            .json(&AuthLoginRequest { username, password })
            .send()
            .await?;

        if response.status().is_success() {
            let token = response.json::<AuthTokenResponse>().await?;
            self.state.set_auth_token(token.token.clone());
            Ok(token)
        } else {
            remote_error(response).await?;
            unreachable!()
        }
    }

    pub fn set_auth_token(&self, token: String) {
        self.state.set_auth_token(token);
    }

    pub fn logout(&self) {
        self.state.clear_auth_token();
    }

    pub fn is_authenticated(&self) -> bool {
        self.state.auth_token().is_some()
    }
}

#[tauri::command]
pub async fn che_api(
    api: tauri::State<'_, TauriApi>,
    request: ApiRequest,
) -> std::result::Result<Value, ApiError> {
    api.dispatch(request).await
}

fn required_id(id: Option<i64>, action: ApiAction) -> ApiResult<i64> {
    id.ok_or_else(|| ApiError::bad_request(format!("id is required for {action:?}")))
}

fn required_payload(payload: Option<Value>, action: ApiAction) -> ApiResult<Value> {
    payload.ok_or_else(|| ApiError::bad_request(format!("payload is required for {action:?}")))
}

fn remote_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .expect("failed to build reqwest client")
}

fn remote_url(state: &AppState, remote_path: &str) -> ApiResult<String> {
    let remote = state.remote_config().ok_or_else(|| {
        ApiError::bad_request("remote resource requires [remote].base_url config")
    })?;
    Ok(format!(
        "{}/{}",
        remote.base_url.trim_end_matches('/'),
        remote_path.trim_start_matches('/')
    ))
}

fn remote_auth_url(state: &AppState) -> ApiResult<String> {
    let remote = state
        .remote_config()
        .ok_or_else(|| ApiError::bad_request("auth requires [remote].base_url config"))?;
    let auth_path = remote.auth_path.as_deref().unwrap_or("/api-token-auth/");

    Ok(format!(
        "{}/{}",
        remote.base_url.trim_end_matches('/'),
        auth_path.trim_start_matches('/')
    ))
}

fn with_token(
    state: &AppState,
    request: reqwest::RequestBuilder,
) -> ApiResult<reqwest::RequestBuilder> {
    let token = state
        .auth_token()
        .ok_or_else(|| ApiError::new("not_authenticated", "authentication token is missing"))?;

    Ok(request.header(reqwest::header::AUTHORIZATION, format!("Token {token}")))
}

async fn remote_json(response: reqwest::Response) -> ApiResult<Value> {
    if response.status().is_success() {
        Ok(response.json::<Value>().await?)
    } else {
        remote_error(response).await
    }
}

async fn remote_error(response: reqwest::Response) -> ApiResult<Value> {
    let status = response.status();
    let detail = response
        .text()
        .await
        .unwrap_or_else(|_| "failed to read remote error response".to_string());
    Err(ApiError::new(
        "remote_error",
        format!("remote request failed with {status}: {detail}"),
    ))
}

fn api_fields<M>(fields: &[Field]) -> Vec<crate::ApiField>
where
    M: Model,
{
    fields
        .iter()
        .filter_map(|field| {
            let schema = M::schema();
            let model_field = schema.columns.iter().find(|model_field| {
                model_field.name == field.source
                    || model_field.field_name == field.source
                    || model_field.name == field.name
                    || model_field.field_name == field.name
            })?;
            Some(crate::ApiField {
                name: field.name.to_string(),
                source: field.source.to_string(),
                ty: model_field.column_type,
                related_model: field
                    .relation
                    .map(|relation| relation.model_name().to_string()),
                ts_type: field.ts_type.map(ToString::to_string),
                input_ts_type: field.input_ts_type.map(ToString::to_string),
                read_only: field.read_only
                    || model_field.primary_key
                    || model_field.auto_now
                    || model_field.auto_now_add,
                write_only: field.write_only,
                required: field.required,
                nullable: field.nullable || model_field.nullable,
                has_default: field.has_default() || model_field.default.is_some(),
            })
        })
        .collect()
}

fn api_filters<M>(filters: &[Filter]) -> Vec<crate::ApiFilter>
where
    M: Model,
{
    filters
        .iter()
        .filter_map(|filter| {
            let schema = M::schema();
            let model_field = schema.columns.iter().find(|model_field| {
                model_field.name == filter.source || model_field.field_name == filter.source
            })?;
            Some(crate::ApiFilter {
                name: filter.query_name(),
                source: filter.source.to_string(),
                ty: model_field.column_type,
                nullable: model_field.nullable,
            })
        })
        .collect()
}
