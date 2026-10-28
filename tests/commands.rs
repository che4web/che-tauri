use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use che_tauri::{ApiError, CommandRequest, CommandRouter, commands::typescript};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SaveInput {
    id: che_orm::Uuid,
    #[serde(rename = "displayName")]
    name: String,
    note: Option<String>,
}

#[derive(Serialize, JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
enum Saved {
    Task {
        id: che_orm::Uuid,
        title: String,
        note: Option<String>,
    },
}

#[tokio::test]
async fn typed_operations_share_dispatch_and_generation_without_runtime_setup() {
    let router = CommandRouter::<AtomicUsize>::new()
        .command("save", |state, input: SaveInput| async move {
            state.fetch_add(1, Ordering::SeqCst);
            Ok(Saved::Task {
                id: input.id,
                title: input.name,
                note: input.note,
            })
        })
        .unwrap()
        .command("logout", |_, (): ()| async { Ok(()) })
        .unwrap()
        .command("denied", |_, (): ()| async {
            Err::<(), _>(ApiError::bad_request("not allowed"))
        })
        .unwrap();
    // Generation requires no state, database, application handle or network.
    let ts = typescript(router.metadata()).unwrap();
    assert!(ts.contains("\"displayName\": string"));
    assert!(ts.contains("\"note\"?: (string | null)"));
    assert!(ts.contains("\"kind\": \"task\""));
    assert!(ts.contains("\"logout\": (payload: null = null): Promise<null>"));
    assert!(!ts.contains("export type null"));
    assert!(ts.contains("invoke('che_invoke', { request:"));

    let state = Arc::new(AtomicUsize::new(0));
    let commands = router.bind(state.clone());
    let id = che_orm::Uuid::now_v7();
    let result = commands
        .dispatch(CommandRequest {
            operation: "save".into(),
            payload: json!({ "id": id, "displayName": "Task" }),
        })
        .await
        .unwrap();
    assert_eq!(
        result,
        json!({ "kind": "task", "data": { "id": id, "title": "Task", "note": null } })
    );
    for payload in [
        json!({ "id": 42, "displayName": "Wrong ID" }),
        json!({"id": id}),
        json!({ "id": id, "displayName": "Unknown field", "extra": true }),
    ] {
        assert_eq!(
            commands
                .dispatch(CommandRequest {
                    operation: "save".into(),
                    payload
                })
                .await
                .unwrap_err()
                .code,
            "bad_request"
        );
    }
    assert_eq!(
        state.load(Ordering::SeqCst),
        1,
        "invalid inputs must never run the handler"
    );
    assert_eq!(
        commands
            .dispatch(CommandRequest {
                operation: "missing".into(),
                payload: json!(null)
            })
            .await
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(
        commands
            .dispatch(CommandRequest {
                operation: "denied".into(),
                payload: json!(null)
            })
            .await
            .unwrap_err()
            .detail,
        "not allowed"
    );
    assert_eq!(
        commands
            .dispatch(serde_json::from_value(json!({"operation": "logout"})).unwrap())
            .await
            .unwrap(),
        json!(null)
    );
}

#[test]
fn duplicate_operations_and_ambiguous_dto_names_fail_explicitly() {
    let router = CommandRouter::<()>::new()
        .command("save", |_, (): ()| async { Ok(()) })
        .unwrap();
    assert!(
        router
            .command("save", |_, (): ()| async { Ok(()) })
            .is_err()
    );
    assert!(
        CommandRouter::<()>::new()
            .command("bad name", |_, (): ()| async { Ok(()) })
            .is_err()
    );

    #[derive(Deserialize, Serialize, JsonSchema)]
    struct DifferentContracts {
        #[serde(skip_serializing)]
        secret: String,
    }
    let router = CommandRouter::<()>::new()
        .command("echo", |_, input: DifferentContracts| async move {
            let _ = &input.secret;
            Ok(input)
        })
        .unwrap();
    assert!(
        typescript(router.metadata())
            .unwrap_err()
            .detail
            .contains("conflicting schema definitions")
    );
}

#[tokio::test]
async fn flattened_enum_keeps_required_common_fields_in_the_client_contract() {
    #[derive(Deserialize, JsonSchema)]
    struct Flattened {
        id: String,
        #[serde(flatten)]
        content: Content,
    }
    #[derive(Deserialize, JsonSchema)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum Content {
        Text { text: String },
        Count { count: i64 },
    }
    let router = CommandRouter::<()>::new()
        .command("flatten", |_, input: Flattened| async move {
            let value = match input.content {
                Content::Text { text } => text,
                Content::Count { count } => count.to_string(),
            };
            Ok(format!("{}:{value}", input.id))
        })
        .unwrap();
    let ts = typescript(router.metadata()).unwrap();
    let flattened = ts
        .lines()
        .find(|line| line.starts_with("export type Flattened ="))
        .unwrap();
    assert!(flattened.contains("\"id\": string"));
    assert!(flattened.contains(" & "));
    assert!(flattened.contains("\"kind\": \"text\""));
    assert!(flattened.contains("\"kind\": \"count\""));
    let commands = router.bind(Arc::new(()));
    assert_eq!(
        commands
            .dispatch(CommandRequest {
                operation: "flatten".into(),
                payload: json!({"id": "1", "kind": "text", "text": "ok"})
            })
            .await
            .unwrap(),
        json!("1:ok")
    );
    assert!(
        commands
            .dispatch(CommandRequest {
                operation: "flatten".into(),
                payload: json!({"kind": "text", "text": "missing id"})
            })
            .await
            .is_err()
    );
}

#[test]
fn collection_types_support_json_values_and_cannot_be_shadowed_by_dtos() {
    #[derive(Serialize, JsonSchema)]
    struct Record {
        name: String,
    }
    #[derive(Serialize, JsonSchema)]
    struct Array {
        name: String,
    }
    #[derive(Serialize, JsonSchema)]
    struct Collections {
        records: Vec<Record>,
        arrays: std::collections::BTreeMap<String, Array>,
        json: std::collections::BTreeMap<String, serde_json::Value>,
    }
    let router = CommandRouter::<()>::new()
        .command("collections", |_, (): ()| async {
            Ok(Collections {
                records: vec![Record {
                    name: "record".into(),
                }],
                arrays: [(
                    "array".into(),
                    Array {
                        name: "array".into(),
                    },
                )]
                .into(),
                json: [("nested".into(), json!({"value": 1}))].into(),
            })
        })
        .unwrap();
    let ts = typescript(router.metadata()).unwrap();
    assert!(ts.contains("export type Record ="));
    assert!(ts.contains("export type Array ="));
    assert!(ts.contains("\"records\": (Record)[]"));
    assert!(ts.contains("\"arrays\": { [key: string]: Array }"));
    assert!(ts.contains("\"json\": { [key: string]: unknown }"));
    assert!(!ts.contains("Array<"));
    assert!(!ts.contains("Record<"));
}

#[test]
fn helper_names_are_aliased_without_colliding_with_existing_dtos() {
    #[derive(Serialize, JsonSchema)]
    struct InvokeError {
        code: String,
    }
    #[derive(Serialize, JsonSchema)]
    struct ApiDtoInvokeError {
        detail: String,
    }
    #[derive(Serialize, JsonSchema)]
    struct Promise {
        error: InvokeError,
        other: ApiDtoInvokeError,
    }
    let router = CommandRouter::<()>::new()
        .command("error_details", |_, (): ()| async {
            Ok(InvokeError {
                code: "test".into(),
            })
        })
        .unwrap()
        .command("promise", |_, (): ()| async {
            Ok(Promise {
                error: InvokeError {
                    code: "test".into(),
                },
                other: ApiDtoInvokeError {
                    detail: "detail".into(),
                },
            })
        })
        .unwrap();
    let ts = typescript(router.metadata()).unwrap();
    assert_eq!(ts.matches("export interface InvokeError").count(), 1);
    assert!(!ts.contains("export type InvokeError ="));
    assert!(ts.contains("export type ApiDtoInvokeError = { \"detail\": string }"));
    assert!(ts.contains("export type ApiDtoInvokeError2 = { \"code\": string }"));
    assert!(ts.contains("export type ApiDtoPromise = { \"error\": ApiDtoInvokeError2; \"other\": ApiDtoInvokeError }"));
    assert!(ts.contains("Promise<ApiDtoInvokeError2>"));
    assert!(ts.contains("Promise<ApiDtoPromise>"));
}

#[test]
fn recursive_dtos_have_one_definition_when_used_as_root_and_nested() {
    #[derive(Serialize, JsonSchema)]
    struct Node {
        value: String,
        children: Vec<Node>,
    }
    #[derive(Serialize, JsonSchema)]
    struct Envelope {
        node: Node,
    }
    let router = CommandRouter::<()>::new()
        .command("node", |_, (): ()| async {
            Ok(Node {
                value: "root".into(),
                children: vec![],
            })
        })
        .unwrap()
        .command("envelope", |_, (): ()| async {
            Ok(Envelope {
                node: Node {
                    value: "nested".into(),
                    children: vec![],
                },
            })
        })
        .unwrap();
    let ts = typescript(router.metadata()).unwrap();
    assert_eq!(ts.matches("export type Node =").count(), 1);
    assert!(ts.contains("\"children\": (Node)[]"));
    assert!(ts.contains("export type Envelope = { \"node\": Node }"));
    assert!(ts.contains("Promise<Node>"));
    assert!(ts.contains("Promise<Envelope>"));
}
