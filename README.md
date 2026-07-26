# che-tauri

Tauri invoke API layer for `che-orm` applications.

`che-tauri` mirrors the app/module/serializer/filter architecture from `che-rest`, but exposes model resources through one Tauri command instead of HTTP routes. It supports local SQLite resources, remote REST-backed resources, generated TypeScript clients, project-local migrations, and token-based remote requests.

## Features

- App/module registry through `InstalledApps` and `AppModule`.
- Local SQLite model CRUD through `ctx.resource`.
- Remote REST proxy resources through `ctx.raw_remote_resource`.
- Remote REST resources with serializer normalization through `ctx.mapped_remote_resource`.
- Serializer metadata for generated frontend response/create/update types.
- Filter metadata for generated list params and local/remote filtering.
- Project-local management CLI for `startapp`, migrations and TypeScript generation.
- Shared auth token storage in `AppState` for remote requests.

## Install

Use it from an app crate with path dependencies:

```toml
[dependencies]
che-orm = { path = "../../che-orm/crates/che-orm" }
che-tauri = { path = "../../che-tauri" }
tauri = "2"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

## Configuration

`AppState::from_config_file` reads TOML config with database and optional remote settings:

```toml
[database]
url = "sqlite://app.sqlite?mode=rwc"

[remote]
base_url = "https://api.example.com"
auth_path = "/api-token-auth/"
```

Equivalent manual setup is also supported:

```rust
use che_orm::SqliteBackend;
use che_tauri::{AppConfig, AppState, DatabaseConfig, RemoteConfig};

# async fn example() -> che_tauri::ApiResult<AppState> {
let config = AppConfig {
    database: DatabaseConfig {
        url: "sqlite://app.sqlite?mode=rwc".to_string(),
    },
    remote: Some(RemoteConfig {
        base_url: "https://api.example.com".to_string(),
        auth_path: Some("/api-token-auth/".to_string()),
    }),
};
let db = SqliteBackend::connect(&config.database.url).await?;
let state = AppState::new(config, db);
# Ok(state)
# }
```

## App Modules

Application code defines installed modules once and reuses that list for both Tauri runtime and management commands.

```rust
pub mod users;

use che_tauri::InstalledApps;

pub fn installed_apps() -> InstalledApps {
    InstalledApps::new().add(users::module())
}
```

Example app module:

```rust
use che_tauri::{AppModule, ModuleContext};

pub struct UsersModule;

pub fn module() -> UsersModule {
    UsersModule
}

impl AppModule for UsersModule {
    fn name(&self) -> &'static str {
        "users"
    }

    fn init(&self, ctx: &mut ModuleContext) {
        ctx.resource::<models::User>(
            "users",
            serializers::user_serializer(),
            filters::user_filterset(),
        );
    }
}
```

## Models

Models are regular `che-orm` models:

```rust
use che_orm::Model;

#[derive(Debug, Clone, Model)]
#[model(table = "users")]
pub struct User {
    #[field(primary_key)]
    pub id: i64,
    pub email: String,
    pub name: String,
    pub is_active: bool,
}
```

## Serializers

Serializers define frontend field names, source mappings, required/nullability rules, defaults, write/read-only fields and related output fields.

```rust
use che_tauri::{Field, ModelSerializer};

use super::models::User;

static USER_FIELDS: &[Field] = &[
    Field::new("id").read_only(),
    Field::new("email"),
    Field::new("name"),
    Field::new("is_active"),
];

pub fn user_serializer() -> ModelSerializer<User> {
    ModelSerializer::new(USER_FIELDS)
}
```

Remote JSON fields can be mapped to local/frontend names:

```rust
Field::new("display_name").source("get_display_name")
```

## Filters

Filters define accepted list query params and generated TypeScript list types.

```rust
use che_tauri::{Filter, FilterSet};

use super::models::User;

static USER_FILTERS: &[Filter] = &[
    Filter::exact("id"),
    Filter::contains("name"),
    Filter::exact("is_active"),
];

pub fn user_filterset() -> FilterSet<User> {
    FilterSet::new(USER_FILTERS)
}
```

Supported lookups include `exact`, `contains`, `gt`, `gte`, `lt`, and `lte`. Generated query names follow Django-style suffixes such as `name__contains` and `id__gte`.

Remote filter helpers:

- `Filter::remote("remote_name")`: forwards a local filter under a different remote query name.
- `Filter::remote_only("name")`: forwards to remote but does not apply locally.
- `filter.local_only()`: applies only after mapped remote results are normalized.
- `filter.remote_disabled()`: accepts local filtering but does not forward remotely.
- `FilterSet::remote_ordering("remote_ordering")`: maps frontend `ordering` to a different remote ordering parameter.

## Resource Types

Local SQLite resource:

```rust
ctx.resource::<models::User>(
    "users",
    serializers::user_serializer(),
    filters::user_filterset(),
);
```

This creates table SQL metadata, registers CRUD invoke handlers and includes the resource in generated TypeScript.

Mapped remote REST resource:

```rust
ctx.mapped_remote_resource::<models::User>(
    "users",
    "/api/users/",
    serializers::user_serializer(),
    filters::user_filterset(),
);
```

This forwards requests to the remote API, normalizes responses through the serializer, applies local-only JSON filters, and includes the resource in generated TypeScript. It does not create a local table.

Raw remote REST resource:

```rust
ctx.raw_remote_resource::<models::User>(
    "users",
    "/api/users/",
    serializers::user_serializer(),
    filters::user_filterset(),
);
```

This proxies remote JSON mostly as-is while still using serializer/filter metadata for generated TypeScript. It does not create a local table.

`ctx.remote_resource` currently delegates to mapped remote behavior.

Remote REST action mapping:

```text
list     GET    <remote_path>
retrieve GET    <remote_path>/<id>/
create   POST   <remote_path>
update   PATCH  <remote_path>/<id>/
delete   DELETE <remote_path>/<id>/
```

## Tauri Runtime Setup

Register `che_api` and manage a built `TauriApi` instance:

```rust
use che_tauri::{AppState, TauriApi, che_api};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let api = tauri::async_runtime::block_on(async {
        let state = AppState::from_config_file("app.toml").await?;
        TauriApi::new(state)
            .install(apps::installed_apps())
            .build()
            .await
    })
    .expect("failed to initialize che-tauri");

    tauri::Builder::default()
        .manage(api)
        .invoke_handler(tauri::generate_handler![che_api])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

`TauriApi::build()` initializes registered modules, applies create-table SQL for local resources, and stores invoke resources by resource name.

## Frontend Usage

Raw invoke request:

```ts
import { invoke } from "@tauri-apps/api/core";

const users = await invoke("che_api", {
  request: {
    resource: "users",
    action: "list",
    params: { name__contains: "Ali", ordering: "-id" },
  },
});
```

Generated API usage:

```ts
import { userApi } from "./generated/api";

const users = await userApi.list({ name__contains: "Ali" });
const user = await userApi.create({ email: "alice@example.com", name: "Alice", is_active: true });
await userApi.update(user.id, { name: "Alicia" });
await userApi.remove(user.id);
```

Generated list responses use this shape:

```ts
interface PaginatedResponse<T> {
  count: number;
  results: T[];
}
```

## Authentication

`TauriApi` supports remote token login when `[remote].auth_path` is configured:

```rust
let response = api.login(username, password).await?;
api.set_auth_token(response.token);
```

Remote resource requests send:

```text
Authorization: Token <token>
```

Applications can expose custom Tauri commands for login/logout/token restore around `TauriApi::login`, `set_auth_token`, `logout`, and `is_authenticated`.

## Management CLI

Application crates should define a project-local `src/bin/manage.rs`:

```rust
use che_tauri::Management;

use my_app::apps;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    Management::new(apps::installed_apps()).run().await
}
```

Create a new app module skeleton:

```bash
cargo run --bin manage -- startapp users
```

Create migrations for one installed app:

```bash
cargo run --bin manage -- makemigrations users
```

Use a custom migration name:

```bash
cargo run --bin manage -- makemigrations users --name add_profile_fields
```

Apply migrations for one app or all apps:

```bash
cargo run --bin manage -- migrate users
cargo run --bin manage -- migrate
```

Override database config:

```bash
cargo run --bin manage -- migrate --database-url 'sqlite://app.sqlite?mode=rwc'
```

Generate TypeScript models and invoke API:

```bash
cargo run --bin manage -- generate-ts --out ../src/generated
```

Generated files:

```text
src/generated/
  api_client.ts
  models.ts
  api.ts
```

## Development

Run checks from this library directory:

```bash
cargo fmt
cargo check
cargo test
```

Useful source files:

- `src/lib.rs`: public exports.
- `src/api.rs`: invoke dispatch, local/remote resource implementations, generated client template.
- `src/module.rs`: app modules, resource registration and API metadata.
- `src/serializer.rs`: serializer fields and JSON validation/normalization.
- `src/filters.rs`: local filters, remote filter mapping and ordering.
- `src/management.rs`: CLI commands, migrations and TypeScript generation.
- `src/state.rs`: app state, database and auth token storage.
- `src/config.rs`: TOML config types.

## Notes

- Local resources require `SqliteModel<Id = i64>`.
- Remote resources still require model metadata so serializers, filters and generated TypeScript can be derived.
- `raw_remote_resource` and `mapped_remote_resource` do not create local SQLite tables.
- Generated TypeScript should be regenerated from metadata rather than edited by hand.
- Migrations are app-scoped under `src/apps/<app>/migrations` in consuming applications.
