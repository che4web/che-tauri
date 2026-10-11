use std::collections::HashMap;

use che_orm::{Database, Model};
use che_tauri::{
    ApiAction, ApiRequest, AppConfig, AppModule, AppState, DatabaseConfig, Field, Filter,
    FilterSet, InstalledApps, ModelSerializer, ModuleContext, TauriApi,
};
use serde_json::{Value, json};

#[derive(Debug, serde::Serialize, Model)]
#[orm(table = "balances")]
struct Balance {
    #[orm(primary_key)]
    id: i64,
    amount: f64,
    optional_amount: Option<f64>,
}

struct Balances;
impl AppModule for Balances {
    fn name(&self) -> &'static str {
        "balances"
    }
    fn init(&self, ctx: &mut ModuleContext) {
        static FIELDS: &[Field] = &[
            Field::new("id").read_only(),
            Field::new("amount"),
            Field::new("optional_amount").nullable(),
        ];
        static FILTERS: &[Filter] = &[Filter::gte("amount")];
        ctx.resource::<Balance>(
            "balances",
            ModelSerializer::new(FIELDS),
            FilterSet::new(FILTERS),
        );
    }
}

fn request(action: ApiAction, payload: Option<Value>) -> ApiRequest {
    ApiRequest {
        resource: "balances".into(),
        action,
        id: None,
        payload,
        params: HashMap::new(),
    }
}

#[tokio::test]
async fn real_crud_filters_and_validation() {
    let state = AppState::new(
        AppConfig {
            database: DatabaseConfig {
                url: ":memory:".into(),
            },
            remote: None,
        },
        Database::connect_in_memory().await.unwrap(),
    );
    let api = TauriApi::new(state)
        .install(InstalledApps::new().add(Balances))
        .build()
        .await
        .unwrap();
    let created = api
        .dispatch(request(
            ApiAction::Create,
            Some(json!({"amount": 12.75, "optional_amount": -0.125})),
        ))
        .await
        .unwrap();
    assert_eq!(created["amount"], 12.75);
    assert_eq!(created["optional_amount"], -0.125);
    let mut list = request(ApiAction::List, None);
    list.params.insert("amount__gte".into(), "12.5".into());
    assert_eq!(api.dispatch(list).await.unwrap()["count"], 1);
    let mut list = request(ApiAction::List, None);
    list.params.insert("amount__gte".into(), "13.5".into());
    assert_eq!(api.dispatch(list).await.unwrap()["count"], 0);
    let mut list = request(ApiAction::List, None);
    list.params.insert("amount__gte".into(), "NaN".into());
    assert!(api.dispatch(list).await.is_err());
    let mut update = request(
        ApiAction::Update,
        Some(json!({"amount": 0.25, "optional_amount": null})),
    );
    update.id = created["id"].as_i64();
    let updated = api.dispatch(update).await.unwrap();
    assert_eq!(updated["amount"], 0.25);
    assert!(updated["optional_amount"].is_null());
    assert!(
        api.dispatch(request(
            ApiAction::Create,
            Some(json!({"amount": "12.75", "optional_amount": null}))
        ))
        .await
        .is_err()
    );
}
