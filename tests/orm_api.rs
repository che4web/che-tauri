use std::collections::HashMap;

use che_orm::{Database, DbEnum, Model};
use che_tauri::{
    ApiAction, ApiRequest, AppConfig, AppModule, AppState, DatabaseConfig, Field, Filter,
    FilterSet, InstalledApps, ModelSerializer, ModuleContext, RelatedModel, TauriApi,
};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, DbEnum)]
enum Status {
    Draft,
    Published,
}

#[derive(Debug, Serialize, Model)]
#[orm(table = "authors")]
struct Author {
    #[orm(primary_key)]
    id: i64,
    #[serde(rename = "displayName")]
    name: String,
}

#[derive(Debug, Serialize, Model)]
#[orm(table = "articles", index("headline"))]
#[serde(rename_all = "camelCase")]
struct Article {
    #[orm(primary_key)]
    id: i64,
    #[orm(db_column = "headline")]
    #[serde(rename = "status")]
    title: String,
    #[serde(rename = "state")]
    status: Status,
    external_id: Option<che_orm::Uuid>,
    #[serde(rename = "owner")]
    author_id: Option<i64>,
    body: Vec<u8>,
    #[serde(with = "che_orm::time::serde::rfc3339")]
    published_at: che_orm::time::OffsetDateTime,
    #[orm(default = "true")]
    active: bool,
    #[orm(auto_now_add)]
    #[serde(with = "che_orm::time::serde::rfc3339")]
    created_at: che_orm::time::OffsetDateTime,
    #[orm(auto_now)]
    #[serde(with = "che_orm::time::serde::rfc3339")]
    updated_at: che_orm::time::OffsetDateTime,
}

fn author_serializer() -> ModelSerializer<Author> {
    static FIELDS: &[Field] = &[Field::new("id").read_only(), Field::new("name")];
    ModelSerializer::new(FIELDS)
}

static AUTHOR: RelatedModel<Author> = RelatedModel::new(author_serializer);
static ARTICLE_FIELDS: &[Field] = &[
    Field::new("id").read_only(),
    Field::new("title").source("headline"),
    Field::new("status"),
    Field::new("external_id").required(false).nullable(),
    Field::new("author_id").nullable(),
    Field::related("author", "author_id", &AUTHOR).nullable(),
    Field::new("body"),
    Field::new("published_at"),
    Field::new("active"),
    Field::new("created_at").read_only(),
    Field::new("updated_at").read_only(),
];
static ARTICLE_FILTERS: &[Filter] = &[
    Filter::contains("title"),
    Filter::exact("status"),
    Filter::exact("external_id"),
    Filter::gte("published_at"),
    Filter::gte("id"),
];

struct Articles;
impl AppModule for Articles {
    fn name(&self) -> &'static str {
        "articles"
    }
    fn init(&self, ctx: &mut ModuleContext) {
        ctx.resource("authors", author_serializer(), FilterSet::new(&[]));
        ctx.resource::<Article>(
            "articles",
            ModelSerializer::new(ARTICLE_FIELDS),
            FilterSet::new(ARTICLE_FILTERS),
        );
    }
}

async fn api() -> TauriApi {
    let state = AppState::new(
        AppConfig {
            database: DatabaseConfig {
                url: ":memory:".to_string(),
            },
            remote: None,
        },
        Database::connect_in_memory().await.unwrap(),
    );
    TauriApi::new(state)
        .install(InstalledApps::new().add(Articles))
        .build()
        .await
        .unwrap()
}

fn request(
    resource: &str,
    action: ApiAction,
    id: Option<i64>,
    payload: Option<Value>,
) -> ApiRequest {
    ApiRequest {
        resource: resource.into(),
        action,
        id,
        payload,
        params: HashMap::new(),
    }
}

#[tokio::test]
async fn crud_filters_relations_and_new_column_types() {
    let api = api().await;
    api.initialize().await.unwrap(); // Repeated startup remains idempotent, including indexes.
    let author = api
        .dispatch(request(
            "authors",
            ApiAction::Create,
            None,
            Some(json!({"name": "Alice"})),
        ))
        .await
        .unwrap();
    let payload = json!({"title": "First", "status": "draft", "author_id": author["id"], "body": [0, 128, 255], "published_at": "2026-10-08T10:20:30Z"});
    let created = api
        .dispatch(request(
            "articles",
            ApiAction::Create,
            None,
            Some(payload.clone()),
        ))
        .await
        .unwrap();
    assert_eq!(created["title"], "First");
    assert_eq!(created["body"], payload["body"]);
    assert_eq!(created["published_at"], payload["published_at"]);
    assert_eq!(created["author"]["name"], "Alice");
    assert_eq!(created["active"], true);
    assert!(created["created_at"].as_str().is_some());
    assert!(created["updated_at"].as_str().is_some());
    let id = created["id"].as_i64().unwrap();
    let model = api.state().db().get::<Article>(id).await.unwrap().unwrap();
    let serialized = ModelSerializer::<Article>::new(ARTICLE_FIELDS).to_json(&model);
    assert_eq!(serialized["id"], id);
    assert_eq!(serialized["title"], "First");
    assert_eq!(serialized["status"], "draft");
    assert_eq!(serialized["author_id"], author["id"]);
    assert_eq!(serialized["created_at"], created["created_at"]);
    assert_eq!(serialized["updated_at"], created["updated_at"]);

    let mut list = request("articles", ApiAction::List, None, None);
    list.params = HashMap::from([
        ("title__contains".into(), "irs".into()),
        ("published_at__gte".into(), "2026-10-01T00:00:00Z".into()),
        ("ordering".into(), "-title".into()),
        ("limit".into(), "1".into()),
    ]);
    let page = api.dispatch(list).await.unwrap();
    assert_eq!(page["count"], 1);
    assert_eq!(page["results"][0], created);

    let mut offset_only = request("articles", ApiAction::List, None, None);
    offset_only.params.insert("offset".into(), "1".into());
    let page = api.dispatch(offset_only).await.unwrap();
    assert_eq!(page["count"], 1);
    assert_eq!(page["results"], json!([]));

    let updated = api
        .dispatch(request(
            "articles",
            ApiAction::Update,
            Some(id),
            Some(json!({"title": "Second", "status": "published"})),
        ))
        .await
        .unwrap();
    assert_eq!(updated["title"], "Second");
    assert_eq!(updated["status"], "published");
    assert_eq!(updated["body"], payload["body"]);
    let external_id = che_orm::Uuid::now_v7().to_string();
    let updated = api
        .dispatch(request(
            "articles",
            ApiAction::Update,
            Some(id),
            Some(json!({"external_id": external_id})),
        ))
        .await
        .unwrap();
    assert_eq!(updated["external_id"], external_id);
    let mut uuid_filter = request("articles", ApiAction::List, None, None);
    uuid_filter.params.insert("external_id".into(), external_id);
    assert_eq!(api.dispatch(uuid_filter).await.unwrap()["count"], 1);
    assert_eq!(
        api.dispatch(request(
            "articles",
            ApiAction::Update,
            Some(id),
            Some(json!({"external_id": "invalid"})),
        ))
        .await
        .unwrap_err()
        .code,
        "bad_request"
    );
    assert_eq!(
        api.dispatch(request("articles", ApiAction::Retrieve, Some(id), None))
            .await
            .unwrap(),
        updated
    );

    assert_eq!(
        api.dispatch(request(
            "articles",
            ApiAction::Update,
            Some(id),
            Some(json!({}))
        ))
        .await
        .unwrap_err()
        .code,
        "bad_request"
    );
    assert_eq!(
        api.dispatch(request(
            "articles",
            ApiAction::Update,
            Some(id),
            Some(json!({"status": "unknown"}))
        ))
        .await
        .unwrap_err()
        .code,
        "bad_request"
    );
    assert_eq!(
        api.dispatch(request("articles", ApiAction::Delete, Some(id), None))
            .await
            .unwrap(),
        json!({"deleted": true})
    );
    for action in [ApiAction::Retrieve, ApiAction::Delete] {
        assert_eq!(
            api.dispatch(request("articles", action, Some(id), None))
                .await
                .unwrap_err()
                .code,
            "not_found"
        );
    }
    assert_eq!(
        api.dispatch(request(
            "articles",
            ApiAction::Update,
            Some(id),
            Some(json!({"title": "Missing"}))
        ))
        .await
        .unwrap_err()
        .code,
        "not_found"
    );
}

#[tokio::test]
async fn remote_cache_upserts_nested_models_and_merges_partial_values() {
    let api = api().await;
    let serializer = ModelSerializer::<Article>::new(ARTICLE_FIELDS);
    let external_id = che_orm::Uuid::now_v7();
    let value = json!({"id": 42, "title": "Cached", "status": "draft", "external_id": external_id, "author_id": 7, "author": {"id": 7, "name": "Bob"}, "body": [1, 2], "published_at": "2026-10-08T00:00:00Z", "active": true, "created_at": "2026-10-08T00:00:00Z", "updated_at": "2026-10-08T00:00:00Z"});
    che_tauri::serializer::upsert_normalized_model(api.state().db(), serializer, value)
        .await
        .unwrap();
    che_tauri::serializer::upsert_normalized_model(
        api.state().db(),
        serializer,
        json!({"id": 42, "title": "Changed"}),
    )
    .await
    .unwrap();
    let model = api.state().db().get::<Article>(42).await.unwrap().unwrap();
    let cached = serializer
        .to_json_async(api.state().db(), &model)
        .await
        .unwrap();
    assert_eq!(cached["title"], "Changed");
    assert_eq!(cached["body"], json!([1, 2]));
    assert_eq!(cached["author"]["name"], "Bob");
    assert_eq!(cached["external_id"], external_id.to_string());
}
