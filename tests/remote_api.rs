mod common;

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use che_orm::{Database, Model};
use che_tauri::{
    ApiAction, ApiRequest, AppConfig, AppModule, AppState, DatabaseConfig, Field, Filter,
    FilterSet, InstalledApps, ModelSerializer, ModuleContext, RemoteConfig, TauriApi,
};
use serde_json::{Value, json};

#[derive(Model, serde::Serialize)]
#[orm(table = "remote_items")]
struct Item {
    #[orm(primary_key)]
    id: i64,
    name: String,
    price: f64,
    quantity: i64,
    #[serde(with = "che_orm::time::serde::rfc3339")]
    updated_at: che_orm::time::OffsetDateTime,
}

#[derive(Model, serde::Serialize)]
#[orm(table = "remote_groups")]
struct Group {
    #[orm(primary_key)]
    id: i64,
    name: String,
}

#[derive(Model, serde::Serialize)]
#[orm(table = "local_notes")]
struct Note {
    #[orm(primary_key)]
    id: i64,
    name: String,
}

static FIELDS: &[Field] = &[
    Field::new("id").read_only(),
    Field::new("name").source("remote_name"),
    Field::new("price"),
    Field::new("quantity"),
    Field::new("updated_at").read_only(),
    Field::new("secret").write_only(),
];
static SIMPLE_FIELDS: &[Field] = &[Field::new("id").read_only(), Field::new("name")];
static FILTERS: &[Filter] = &[
    Filter::gt("price").local_only(),
    Filter::gte("price").local_only(),
    Filter::lt("price").local_only(),
    Filter::lte("price").local_only(),
    Filter::gte("quantity").local_only(),
    Filter::gte("updated_at").local_only(),
];

struct Resources;
impl AppModule for Resources {
    fn name(&self) -> &'static str {
        "test"
    }

    fn schema(&self) -> che_orm::SchemaSet {
        che_orm::SchemaSet::new()
            .model::<Item>()
            .model::<Group>()
            .model::<Note>()
    }
    fn init(&self, ctx: &mut ModuleContext) {
        ctx.mapped_remote_resource::<Item>(
            "mapped",
            "/items/",
            ModelSerializer::new(FIELDS),
            FilterSet::new(FILTERS),
        );
        ctx.raw_remote_resource::<Item>(
            "raw",
            "/items/",
            ModelSerializer::new(FIELDS),
            FilterSet::new(FILTERS),
        );
        ctx.cached_mapped_remote_resource::<Item>(
            "cached",
            "/items/",
            ModelSerializer::new(FIELDS),
            FilterSet::new(FILTERS),
        );
        ctx.cached_mapped_remote_resource::<Group>(
            "groups",
            "/groups/",
            ModelSerializer::new(SIMPLE_FIELDS),
            FilterSet::new(&[]),
        );
        ctx.resource::<Note>(
            "notes",
            ModelSerializer::new(SIMPLE_FIELDS),
            FilterSet::new(&[]),
        );
    }
}

fn remote_config(url: &str) -> RemoteConfig {
    RemoteConfig {
        base_url: url.into(),
        auth_path: None,
    }
}

async fn api(url: &str) -> TauriApi {
    let state = AppState::new(
        AppConfig {
            database: DatabaseConfig {
                url: ":memory:".into(),
            },
            remote: Some(remote_config(url)),
        },
        Database::connect_in_memory().await.unwrap(),
    );
    let apps = InstalledApps::new().add(Resources);
    common::migrate(state.db(), &apps).await;
    let api = TauriApi::new(state).install(apps).build().await.unwrap();
    api.set_auth_token("user-A".into());
    api
}

fn request(resource: &str, action: ApiAction, payload: Option<Value>) -> ApiRequest {
    ApiRequest {
        resource: resource.into(),
        action,
        id: Some(1),
        params: HashMap::new(),
        payload,
    }
}

fn item(name: &str) -> Value {
    json!({"id": 1, "remote_name": name, "price": 20.5, "quantity": 10,
        "updated_at": "2026-10-08T10:00:00Z", "secret": "hidden"})
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0, "request ended before its body");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .map(|length| length.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }
}

fn respond(stream: &mut TcpStream, body: Value) {
    let body = body.to_string();
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
}

fn accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "expected HTTP request did not arrive"
                );
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("accept failed: {error}"),
        }
    }
}

fn server(responses: Vec<Value>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        responses
            .into_iter()
            .map(|body| {
                let mut stream = accept(&listener);
                let request = read_request(&mut stream);
                respond(&mut stream, body);
                request
            })
            .collect()
    });
    (url, handle)
}

#[tokio::test]
async fn mapped_mutations_use_response_serializer_while_raw_preserves_json() {
    let (url, server) = server(vec![item("Alice"); 4]);
    let api = api(&url).await;
    for resource in ["mapped", "raw"] {
        for action in [ApiAction::Create, ApiAction::Update] {
            let actual = api
                .dispatch(request(resource, action, Some(json!({"name": "Alice"}))))
                .await
                .unwrap();
            if resource == "mapped" {
                assert_eq!(actual["name"], "Alice");
                assert!(actual.get("remote_name").is_none());
                assert!(actual.get("secret").is_none());
            } else {
                assert_eq!(actual, item("Alice"));
            }
        }
    }
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("POST /items/ HTTP/1.1"));
    assert!(requests[1].starts_with("PATCH /items/1/ HTTP/1.1"));
}

#[test]
fn json_range_filters_compare_numbers_and_timestamp_instants() {
    let filters = FilterSet::<Item>::new(FILTERS);
    let rows = json!({"count": 3, "results": [
        {"id": 1, "price": 1.5}, {"id": 2, "price": 10.5}, {"id": 3, "price": 20.5}
    ]});
    for (lookup, ids) in [
        ("gt", vec![3]),
        ("gte", vec![2, 3]),
        ("lt", vec![1]),
        ("lte", vec![1, 2]),
    ] {
        let params = HashMap::from([(format!("price__{lookup}"), "10.5".into())]);
        let actual = filters.apply_json(rows.clone(), &params);
        assert_eq!(
            actual["results"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            ids
        );
        assert_eq!(actual["count"], ids.len());
    }
    // Values above JS's exact integer range must not be compared as f64.
    let actual = filters.apply_json(json!({"results": [
        {"quantity": 9007199254740992_i64}, {"quantity": 9007199254740993_i64}, {"quantity": null}
    ]}), &HashMap::from([("quantity__gte".into(), "9007199254740993".into())]));
    assert_eq!(
        actual["results"],
        json!([{"quantity": 9007199254740993_i64}])
    );
    let actual = filters.apply_json(
        json!({"results": [
            {"updated_at": "2026-10-08T12:00:00+03:00"},
            {"updated_at": "2026-10-08T10:00:00Z"},
            {"updated_at": "invalid"}
        ]}),
        &HashMap::from([("updated_at__gte".into(), "2026-10-08T10:00:00Z".into())]),
    );
    assert_eq!(
        actual["results"],
        json!([{"updated_at": "2026-10-08T10:00:00Z"}])
    );
}

#[tokio::test]
async fn mapped_list_applies_local_only_ranges_without_forwarding_them() {
    let mut cheap = item("cheap");
    cheap["id"] = json!(2);
    cheap["price"] = json!(1.5);
    let (url, server) = server(vec![
        json!({"count": 2, "results": [cheap, item("expensive")]}),
    ]);
    let api = api(&url).await;
    let mut request = request("mapped", ApiAction::List, None);
    request.params.insert("price__gte".into(), "10.5".into());
    let actual = api.dispatch(request).await.unwrap();
    assert_eq!(actual["count"], 1);
    assert_eq!(actual["results"][0]["name"], "expensive");
    assert!(server.join().unwrap()[0].starts_with("GET /items/ HTTP/1.1"));
}

#[tokio::test]
async fn invalid_local_only_filter_is_rejected_before_remote_request() {
    let api = api("http://127.0.0.1:1").await;
    for resource in ["mapped", "cached"] {
        for (key, value) in [
            ("price__gte", "NaN"),
            ("quantity__gte", "abc"),
            ("updated_at__gte", "yesterday"),
        ] {
            let mut request = request(resource, ApiAction::List, None);
            request.params.insert(key.into(), value.into());
            assert_eq!(api.dispatch(request).await.unwrap_err().code, "bad_request");
        }
    }
}

#[tokio::test]
async fn cache_is_invalidated_between_sessions_and_servers_without_deleting_local_models() {
    let (url, server) = server(vec![item("A"), item("B"), json!([item("C")])]);
    let api = api(&url).await;
    let get = || request("cached", ApiAction::Retrieve, None);
    // Rows left by a previous process have no ownership in this session.
    api.state().db().execute_sql(
        "INSERT INTO remote_items VALUES (99, 'previous process', 1, 1, '2026-10-08T10:00:00Z')",
        vec![],
    ).await.unwrap();
    assert_eq!(api.dispatch(get()).await.unwrap()["name"], "A");
    assert_eq!(api.state().db().count::<Item>().await.unwrap(), 1);
    // A repeated request and setting the same token keep the cache usable.
    api.set_auth_token("user-A".into());
    assert_eq!(api.dispatch(get()).await.unwrap()["name"], "A");
    api.state()
        .db()
        .execute_sql(
            "INSERT INTO remote_groups VALUES (1, 'private A group')",
            vec![],
        )
        .await
        .unwrap();
    api.state()
        .db()
        .execute_sql("INSERT INTO local_notes VALUES (1, 'local note')", vec![])
        .await
        .unwrap();

    // AppState clones share the session generation and cache lock.
    api.state().clone().set_auth_token("user-B");
    assert_eq!(api.dispatch(get()).await.unwrap()["name"], "B");
    assert_eq!(api.state().db().count::<Group>().await.unwrap(), 0);
    assert_eq!(api.state().db().count::<Note>().await.unwrap(), 1);
    api.logout();
    for action in [
        ApiAction::Retrieve,
        ApiAction::List,
        ApiAction::Create,
        ApiAction::Update,
        ApiAction::Delete,
    ] {
        assert_eq!(
            api.dispatch(request("cached", action, Some(json!({}))))
                .await
                .unwrap_err()
                .code,
            "not_authenticated"
        );
    }
    api.set_auth_token("user-C".into());
    assert_eq!(
        api.dispatch(request("cached", ApiAction::List, None))
            .await
            .unwrap()["results"][0]["name"],
        "C"
    );
    let requests = server.join().unwrap();
    for (request, token) in requests.iter().zip(["user-A", "user-B", "user-C"]) {
        assert!(
            request
                .to_ascii_lowercase()
                .contains(&format!("authorization: token {token}").to_ascii_lowercase())
        );
    }

    let (new_url, new_server) = self::server(vec![item("new server")]);
    api.state().set_remote_config(Some(remote_config(&new_url)));
    assert!(!api.is_authenticated());
    assert_eq!(
        api.dispatch(get()).await.unwrap_err().code,
        "not_authenticated"
    );
    api.set_auth_token("user-C".into());
    assert_eq!(api.dispatch(get()).await.unwrap()["name"], "new server");
    new_server.join().unwrap();
    assert_eq!(api.state().db().count::<Note>().await.unwrap(), 1);
}

#[tokio::test]
async fn response_from_previous_session_is_rejected_and_not_reused() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (arrived_tx, arrived_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut first = accept(&listener);
        read_request(&mut first);
        arrived_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        respond(&mut first, item("old session"));
        let mut second = accept(&listener);
        read_request(&mut second);
        respond(&mut second, item("new session"));
    });
    let api = std::sync::Arc::new(api(&url).await);
    let old = {
        let api = api.clone();
        tokio::spawn(async move {
            api.dispatch(request("cached", ApiAction::Retrieve, None))
                .await
        })
    };
    arrived_rx.await.unwrap();
    api.logout();
    api.set_auth_token("new-token".into());
    release_tx.send(()).unwrap();
    assert_eq!(old.await.unwrap().unwrap_err().code, "not_authenticated");
    assert_eq!(
        api.dispatch(request("cached", ApiAction::Retrieve, None))
            .await
            .unwrap()["name"],
        "new session"
    );
    server.join().unwrap();
}
