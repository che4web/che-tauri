# che-tauri

Tauri invoke API layer for `che-orm` applications.

`che-tauri` mirrors the app/module/serializer/filter architecture from `che-rest`.
`che_api` exposes local SQLite resources and remote REST resources with i64 IDs.
The optional `sync` feature adds a separate `che_model_sync` command for durable
offline synchronization of UUID models. Applications can add their own Tauri
commands and generate TypeScript clients for the framework APIs.

For a complete server and desktop client, start with
[the Todo example](examples/todo-sync/README.md).

## Features

- App/module registry through `InstalledApps` and `AppModule`.
- Local SQLite model CRUD through `ctx.resource`.
- Remote REST proxy resources through `ctx.raw_remote_resource`.
- Remote REST resources with serializer normalization through `ctx.mapped_remote_resource`.
- Serializer metadata for generated frontend response/create/update types.
- Filter metadata for generated list params and local/remote filtering.
- Project-local management CLI for `startapp`, migrations and TypeScript generation.
- Shared auth token storage in `AppState` for remote requests.
- Optional protocol-2 offline synchronization through `TauriSync` and `che_model_sync`.

## Install

Use it from an app crate with path dependencies:

```toml
[dependencies]
che-orm = { version = "0.3", path = "../../che-orm" }
che-tauri = { path = "../../che-tauri" }
tauri = "2"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

The checked-out framework uses sibling path dependencies. Keep direct application
dependencies on the same `che-orm` checkout so model and database types come from
one crate instance. Alternatively import ORM types through `che_tauri::che_orm`.
Add `features = ["sync"]` to the `che-tauri` dependency for offline synchronization;
ordinary resource CRUD does not require that feature.

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
use che_orm::Database;
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
let db = Database::connect("app.sqlite").await?;
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

    fn schema(&self) -> che_orm::SchemaSet {
        che_orm::SchemaSet::new().model::<models::User>()
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

Models use the `che-orm` 0.3 API and derive `serde::Serialize` for response serialization:

```rust
use che_orm::Model;

#[derive(Debug, Clone, serde::Serialize, Model)]
#[orm(table = "users")]
pub struct User {
    #[orm(primary_key)]
    pub id: i64,
    pub email: String,
    pub name: String,
    pub is_active: bool,
}
```

Resource serializers read values through `Model::field_values()`, independently
of `#[serde(rename)]` and `#[serde(rename_all)]` on models. `Field::source` resolves
Rust field names and physical SQL column names; `Field::name` determines the
response key. Timestamps are emitted as RFC 3339 strings, and binary fields are
JSON byte arrays.
`f64` and `Option<f64>` fields use SQLite `REAL`, JSON/TypeScript numbers and
numeric filters; remote cache writes preserve fractional values.
UUID fields use JSON/TypeScript strings and support exact filters. Invoke resource
IDs and related serializer IDs remain `i64` in `che_api`. Synchronized UUID
models use `TauriSync` and application-owned CRUD commands, as shown in the
[Todo client](examples/todo-sync/client/src-tauri/src/main.rs).
`AppState::from_config_file` accepts SQLite file paths, `file:` URIs and the
`sqlite://...` configuration syntax. Query parameters are passed to SQLite:
`mode=ro` opens read-only, `mode=rw` requires an existing file, and `mode=rwc`
allows creation. In-memory databases, including `mode=memory` URIs, use one
pooled connection. Manual `Database::connect` accepts a file path or SQLite
`file:` URI.

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

Declare the model separately in `AppModule::schema()`. This registers CRUD invoke handlers and generated TypeScript metadata; it does not create tables.

Mapped remote REST resource:

```rust
ctx.mapped_remote_resource::<models::User>(
    "users",
    "/api/users/",
    serializers::user_serializer(),
    filters::user_filterset(),
);
```

This forwards requests to the remote API, normalizes list/retrieve/create/update responses through the serializer, applies local-only JSON filters, and includes the resource in generated TypeScript. Source mappings and write-only exclusions apply consistently to all response actions. It does not create a local table.

Local-only filters support the same `exact`, `contains`, `gt`, `gte`, `lt`, and `lte`
lookups as local resources. Range comparisons use the model field type: integers
retain their precision, real fields use numeric comparisons, and RFC 3339 timestamps
are compared as instants. Invalid local-only filter values return `bad_request`
before a remote request is sent.

Cached mapped remote resource:

```rust
ctx.cached_mapped_remote_resource::<models::User>(
    "users",
    "/api/users/",
    serializers::user_serializer(),
    filters::user_filterset(),
);
```

Declare its local table in `AppModule::schema()` and create it through compiled
migrations. The resource uses cached records before requesting the remote API. List requests accept `force_remote: true` to bypass the cache. All
cached actions require an authentication token, including local cache hits.

Cached tables belong to the current remote session. Changing the token, logging
out, or changing the remote configuration invalidates the cache; all registered
cached tables are cleared together before the first cached action in a new
authenticated session. Ordinary local resource tables are preserved. Setting the
same token again keeps the current cache. On process restart, existing cached rows
are also cleared on first authenticated cache access. Responses still in flight
when the session changes are rejected and cannot be reused by the new session.

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
use che_tauri::{AppState, TauriApi};

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
        .invoke_handler(tauri::generate_handler![che_tauri::che_api])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

`install()` only stores the module list. `build()` collects the declared schemas,
registers invoke handlers through `init()`, validates registration, then calls all
`subscribe()` hooks and all `start()` hooks in installation order. Both hooks
have empty defaults. The same phases are used by `che-rest`.

`build()` performs no schema writes or automatic migrations. Apply compiled
migrations explicitly before runtime startup. `schema()` describes local/cache
models with `.model::<M>()` and sync models with `.client_model::<M>()`.
Remote-only modules can return `SchemaSet::new()`.

This changes the earlier module API: see [AppModule migration](docs/app-modules.md)
for removed context methods, existing databases and consumer updates.

Use fully qualified paths for commands exported by another crate so Tauri can
resolve their generated wrapper macros. When using both managed states, register
`che_tauri::che_api`, `che_tauri::che_model_sync`, and application commands in
one `generate_handler!` list. `TauriSync` must be opened after explicit compiled
client migrations; resource registration never adds startup CREATE TABLE statements.

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

`AppState::set_remote_config` clears the authentication token when the remote
configuration changes. Authenticate again against the new server before using
remote resources.

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

New migrations are Rust files using `che_orm::Migration` operations. The command
updates `src/apps/users/migrations/mod.rs`. Register them in the app module:

```rust
pub mod migrations;

// Inside impl AppModule for UsersModule:
fn migrations(&self) -> Vec<che_orm::Migration> {
    migrations::migrations()
}
```

Rebuild `manage` after generation so the new migration is compiled. `startapp`
creates this registration automatically. Migration discovery and numbering use
only `.rs` files, excluding `mod.rs`. Schema state and migration history are
managed by `che-orm` through the registered migration graph.

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
  sync.ts          # only when the management binary enables feature "sync"
```

`sync.ts` exports `modelSync` and typed status/conflict/resolution messages.
Its source template is `src/sync_client.ts`; ordinary resource API templates
live in `src/management.rs`. Update these sources and regenerate clients rather
than editing application files under `generated/`.

## Development

Run checks from this library directory:

```bash
cargo fmt --check
cargo check
cargo test
cargo check --features sync
cargo test --features sync
cargo clippy --all-targets --features sync
```

HTTP tests bind ephemeral loopback ports. The Todo example is a separate Cargo
workspace, so the commands above do not build or test it. Verify it separately:

```bash
cargo check --manifest-path examples/todo-sync/Cargo.toml --workspace
cargo test --manifest-path examples/todo-sync/Cargo.toml -p todo-sync-server -p todo-sync-client --no-default-features
cargo fmt --manifest-path examples/todo-sync/Cargo.toml --all --check
```

Useful source files:

- `src/lib.rs`: public exports.
- `src/api.rs`: invoke dispatch and local/remote resource implementations.
- `src/module.rs`: app modules, resource registration and API metadata.
- `src/serializer.rs`: serializer fields and JSON validation/normalization.
- `src/filters.rs`: local filters, remote filter mapping and ordering.
- `src/management.rs`: CLI commands, migrations and TypeScript generation.
- `src/state.rs`: app state, database and auth token storage.
- `src/config.rs`: TOML config types.
- `src/sync.rs`: optional managed synchronization state, command and invoke DTOs.
- `src/sync_client.ts`: canonical TypeScript synchronization client template.

## Notes

- Ordinary resource models require `Model + serde::Serialize + Send + Sync + 'static` and i64 IDs for `che_api`; synchronized models use UUID IDs.
- Remote resources still require model metadata so serializers, filters and generated TypeScript can be derived.
- `raw_remote_resource` and `mapped_remote_resource` do not create local SQLite tables.
- Generated TypeScript should be regenerated from metadata rather than edited by hand.
- Migrations are app-scoped under `src/apps/<app>/migrations` in consuming applications.

## Offline synchronization (protocol 2)

Runnable Todo server and desktop client: [examples/todo-sync](examples/todo-sync/README.md).

Current protocol 2 grants whole-model access. Objects shared through per-object
permissions, including grant/revocation delivery, are the
[planned next stage](../che-rest/docs/OBJECT_SYNC_PLAN.md), **not implemented**
by `TauriSync`. The Todo example does not demonstrate object-level authorization.

Responsibilities are split across the crates:

| Crate | Responsibility |
| --- | --- |
| `che-orm` | Models, queries, transactions, role-specific schemas and compiled migrations |
| `che-sync-protocol` | Transport-independent mutation, journal and conflict messages |
| `che-sync` | Capture, current/shadow data, immutable outbox, cursors and merge |
| `che-rest` | Server REST endpoints and the protocol-2 HTTP transport |
| `che-tauri` | Managed sync state, invoke commands and frontend client generation |

The sync integration accepts any `ModelLogTransport`. Existing ordinary remote
resources still use `che-tauri`'s REST proxy implementation; their cache is
separate from durable synchronized data.

Enable `che-tauri = { path = "../che-tauri", features = ["sync"] }`.
`TauriSync` integrates the existing `che-sync::ModelLogClient`; it exposes
`pull`, `flush`, `sync_once`, `conflicts` and `resolve` to Rust and Tauri invoke.
The engine owns persistent current/shadow data, outbox, immutable submitted
payloads, model cursors and conflict resolution. Clones share one client and its
locks. Open one client per database; independent concurrent clients/processes
against the same file are unsupported.

Declare synchronized models with UUID primary keys, `#[orm(sync)]` and stable
`sync_name` values. Register them in FK parent-first order in app modules:

```rust,ignore
fn schema(&self) -> che_orm::SchemaSet {
    che_orm::SchemaSet::new().client_model::<models::Note>()
}

fn init(&self, _ctx: &mut ModuleContext) {
    // Register invoke resources here if this module has any.
}
```

`AppModule::schema()` registers the **client** role for `makemigrations`, including
sync service tables. `InstalledApps::sync_registry()` reads these schemas without
calling `init()`. Generate, review and apply compiled migrations before
opening the client. This registration does not run startup CREATE TABLE SQL.
Existing local/remote resources and their i64 invoke IDs remain separate; expose
UUID CRUD through application commands calling `TauriSync::save`, `delete` and
`database`. Do not register the same sync table as a cached remote resource:
authentication cache invalidation must never clear durable offline edits.
`TauriApi::build()` rejects ordinary local/cached resource tables declared as
sync models, and local tables missing from the schema. Keep separately configured
runtimes and related cascade effects isolated as well.

The application supplies a `che_sync::ModelLogTransport`. For the existing REST
protocol use `che_rest::sync_http::{RestModelTransport, RestResource}` with
`che-rest`'s `sync-http` feature. HTTP routing, serializer mapping and retry payload
encoding stay in that adapter; `che-tauri` does not duplicate the wire protocol.
Use the server's ORM serializer type for each `RestResource` mapping.
`che-rest` currently uses reqwest 0.12, so give its transport a matching client
(for example an application dependency named `reqwest_sync` with package
`reqwest`, version `0.12`, features `json,rustls-tls`, default features disabled).

```rust,ignore
use che_tauri::TauriSync;
use che_rest::sync_http::{RestModelTransport, RestResource};

// `database` has already had the installed modules' compiled migrations applied.
let transport = RestModelTransport::new(
    "https://host/api",
    authenticated_http_client, // reqwest 0.12: configure credentials and timeout
    vec![RestResource::new::<Note, NoteSerializer>("notes")?],
)?;
let sync = TauriSync::open(database, apps.sync_registry()?, transport).await?;
// Optional: initialize journals while online. Opening itself performs no network IO.
// sync.pull().await?;

tauri::Builder::default()
    .manage(sync)
    .invoke_handler(tauri::generate_handler![che_tauri::che_model_sync])
    // Add che_api and application UUID CRUD commands to this same handler list.
    .run(tauri::generate_context!())?;
```

Use `TauriSync::from_engine(client.engine(transport).with_page_size(50)?)` to
customize page size. Save offline edits through the managed client's `save` or
its configured `database()` handle, so capture remains active. Saving through
another unconfigured database handle bypasses sync capture.

With the `sync` feature, `generate-ts` also writes `sync.ts`:

```ts
import { modelSync } from "./generated/sync";

const report = await modelSync.syncOnce();
if (report.conflicts > 0 || report.blocked) {
  const conflicts = await modelSync.conflicts();
  // Show the competing versions and let the user choose.
  // await modelSync.resolve(conflict.model, conflict.id, { strategy: "remote" });
}
// Schedule another bounded step when appropriate; inspect report.has_more.
```

Raw invoke uses `invoke("che_model_sync", { request: { action: "pull" } })`.
Responses are tagged as `{ kind: "report", data: ... }`,
`{ kind: "conflicts", data: [...] }` or `{ kind: "resolved" }`.
Resolution strategies are `local`, `remote`, and `record` with typed protocol
values (or `record: null` for deletion). Integer values and revisions are decimal
strings to preserve i64 precision; record values distinguish SQL NULL from JSON
null. UUIDs are strings. Errors use the usual `{ code, detail }` shape.
The `record` strategy requires an explicit `record` property: a missing property
is invalid; an explicit `null` chooses deletion.

No background loop starts implicitly. Pull downloads one page per model; repeat
while `has_more` is true. Flush sends at most one coalesced record per model and
retries submitted operations first. `sync_once` settles pending submissions,
pulls journals, then sends new changes when caught up. Stop automatic retry loops
on conflicts/blocked results and use the application's retry policy for network
errors. Transport/authentication failures leave submitted requests pending.

`TauriSync` is independent of `AppState`'s remote response cache and auth token.
The supplied transport owns credentials. Login/logout must never delete the
outbox or silently rebind a database with pending writes to another account or
server. Keep a separate persistent sync database/runtime for each server/account;
recreate the authenticated transport for the same account when credentials renew.
Keep existing legacy sync databases on their original protocol until explicitly
migrated. Opening protocol 2 does not convert them or create application tables.
