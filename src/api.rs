use std::{collections::HashMap, marker::PhantomData};

use async_trait::async_trait;
use che_orm::SqliteModel;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppState, Field, Filter, FilterSet, ModelSerializer,
    error::{ApiError, ApiResult},
    module::InstalledApps,
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

pub struct RemoteInvokeSet<M> {
    resource: &'static str,
    remote_path: &'static str,
    _model: PhantomData<M>,
}

impl<M> ModelInvokeSet<M>
where
    M: SqliteModel<Id = i64>,
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

impl<M> RemoteInvokeSet<M>
where
    M: SqliteModel<Id = i64>,
{
    pub fn new(
        resource: &'static str,
        remote_path: &'static str,
    ) -> Self {
        Self {
            resource,
            remote_path,
            _model: PhantomData,
        }
    }
}

#[async_trait]
impl<M> InvokeResource for ModelInvokeSet<M>
where
    M: SqliteModel<Id = i64>,
{
    fn resource(&self) -> &'static str {
        self.resource
    }

    async fn list(&self, state: &AppState, params: HashMap<String, String>) -> ApiResult<Value> {
        let count_params = params
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "limit" | "offset" | "ordering"))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<HashMap<_, _>>();
        let count = self
            .filterset
            .apply(M::objects(state.db()).query(), &count_params)?
            .count()
            .await?;
        let query = self
            .filterset
            .apply(M::objects(state.db()).query(), &params)?;
        let models = query.all().await?;
        let mut results = Vec::with_capacity(models.len());
        for model in &models {
            results.push(self.serializer.to_json_async(state.db(), model).await?);
        }

        Ok(json!({
            "count": count,
            "results": results,
        }))
    }

    async fn retrieve(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        let model = M::objects(state.db()).get(id).await?;
        Ok(self.serializer.to_json_async(state.db(), &model).await?)
    }

    async fn create(&self, state: &AppState, payload: Value) -> ApiResult<Value> {
        let mut create = M::objects(state.db()).create();

        for (field, value) in self.serializer.create_values(payload)? {
            create = create.set(field, value);
        }

        let model = create.execute().await?;
        Ok(self.serializer.to_json_async(state.db(), &model).await?)
    }

    async fn update(&self, state: &AppState, id: i64, payload: Value) -> ApiResult<Value> {
        let mut update = M::objects(state.db()).update_fields(id);

        for (field, value) in self.serializer.update_values(payload)? {
            update = update.set(field, value);
        }

        let model = update.execute().await?;
        Ok(self.serializer.to_json_async(state.db(), &model).await?)
    }

    async fn delete(&self, state: &AppState, id: i64) -> ApiResult<Value> {
        M::objects(state.db()).get(id).await?;
        M::objects(state.db()).delete(id).await?;
        Ok(json!({ "deleted": true }))
    }
}

#[async_trait]
impl<M> InvokeResource for RemoteInvokeSet<M>
where
    M: SqliteModel<Id = i64>,
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
                "{}/{}",
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
                "{}/{}",
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
                "{}/{}",
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
        M: SqliteModel<Id = i64>,
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
        M: SqliteModel<Id = i64>,
    {
        Self {
            resource,
            fields: api_fields::<M>(serializer.fields()),
            filters: api_filters::<M>(filterset.filters()),
            invoke_resource: RegisteredResource::new(Box::new(RemoteInvokeSet::<M>::new(
                resource, remote_path,
            ))),
        }
    }
}

#[derive(Debug)]
pub struct TauriApi {
    state: AppState,
    resources: HashMap<String, RegisteredResource>,
    sql: Vec<String>,
}

impl TauriApi {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            resources: HashMap::new(),
            sql: Vec::new(),
        }
    }

    pub fn install(mut self, apps: InstalledApps) -> Self {
        let mut ctx = crate::ModuleContext::new();
        for module in apps.iter() {
            module.init(&mut ctx);
        }
        let (sql, resources) = ctx.into_parts();
        self.sql.extend(sql);
        self.resources.extend(
            resources
                .into_iter()
                .map(|resource| (resource.resource.to_string(), resource.invoke_resource)),
        );
        self
    }

    pub async fn initialize(&self) -> ApiResult<()> {
        for sql in &self.sql {
            self.state.db().apply_sql(sql).await?;
        }
        Ok(())
    }

    pub async fn build(self) -> ApiResult<Self> {
        self.initialize().await?;
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
    reqwest::Client::new()
}

fn remote_url(state: &AppState, remote_path: &str) -> ApiResult<String> {
    let remote = state.config.remote.as_ref().ok_or_else(|| {
        ApiError::bad_request("remote resource requires [remote].base_url config")
    })?;
    Ok(format!(
        "{}/{}",
        remote.base_url.trim_end_matches('/'),
        remote_path.trim_matches('/')
    ))
}

fn remote_auth_url(state: &AppState) -> ApiResult<String> {
    let remote = state
        .config
        .remote
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("auth requires [remote].base_url config"))?;
    let auth_path = remote.auth_path.as_deref().unwrap_or("/api-token-auth/");

    Ok(format!(
        "{}/{}",
        remote.base_url.trim_end_matches('/'),
        auth_path.trim_matches('/')
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
    M: SqliteModel<Id = i64>,
{
    fields
        .iter()
        .filter_map(|field| {
            let model_field = M::fields()
                .iter()
                .find(|model_field| model_field.db_name == field.source)?;
            Some(crate::ApiField {
                name: field.name.to_string(),
                source: field.source.to_string(),
                ty: model_field.ty,
                related_model: field
                    .relation
                    .map(|relation| relation.model_name().to_string()),
                read_only: field.read_only,
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
    M: SqliteModel<Id = i64>,
{
    filters
        .iter()
        .filter_map(|filter| {
            let model_field = M::fields()
                .iter()
                .find(|model_field| model_field.db_name == filter.source)?;
            Some(crate::ApiFilter {
                name: filter.query_name(),
                source: filter.source.to_string(),
                ty: model_field.ty,
                nullable: model_field.nullable,
            })
        })
        .collect()
}
