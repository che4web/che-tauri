# AGENTS.md

## Project Overview

`che-tauri` is a Rust library that exposes `che-orm` model resources to Tauri frontends through `che_api`, with optional protocol-2 synchronization through the separate `che_model_sync` command.

It mirrors the `che-rest` app/module/serializer/filter architecture, but dispatches model actions through `che_api` instead of HTTP routes. It is used by `ewa-mobile/src-tauri`.

The root is one library crate. `examples/todo-sync` is an independent Cargo workspace with a shared model, REST server and desktop client; root checks do not cover it. Read its README for launch commands. Keep sibling path dependencies aligned so applications use one `che-orm` crate instance.

## Stack

- Rust edition 2024.
- Tauri 2 invoke command integration.
- `che-orm` 0.3 SQLite async `Database`, models, query ASTs and typed query builders.
- `reqwest` for remote REST resources.
- `serde`/`serde_json` for request and response payloads.
- `clap` for project-local management commands.
- `toml` for app config loading.

## Important Files

- `Cargo.toml`: library dependencies.
- `src/lib.rs`: public exports.
- `src/api.rs`: `TauriApi`, `che_api`, local and remote invoke resources, auth login.
- `src/module.rs`: `AppModule`, `InstalledApps`, `ModuleContext`, resource registration metadata.
- `src/serializer.rs`: `Field`, `ModelSerializer`, related serializers, JSON validation and remote normalization.
- `src/filters.rs`: `Filter`, `FilterSet`, lookup parsing, local filters, remote filter forwarding.
- `src/management.rs`: `startapp`, `makemigrations`, `migrate`, `generate-ts`, ordinary TypeScript templates.
- `src/state.rs`: `AppState`, SQLite `Database`, auth token storage.
- `src/config.rs`: `AppConfig`, `DatabaseConfig`, `RemoteConfig`.
- `src/error.rs`: `ApiError` and `ApiResult`.
- `src/sync.rs`: feature-gated `TauriSync`, sync command, requests, responses and status DTOs.
- `src/sync_client.ts`: canonical TypeScript synchronization client template.
- `tests/model_sync.rs`: retry after reopening, conflicts, input validation and journal pagination.
- `tests/remote_api.rs`: remote response mapping, typed local filters and session cache isolation.
- `examples/todo-sync/server/tests/two_clients.rs`: two persistent clients against the real HTTP protocol.

## Public Concepts

- `AppState`: config, SQLite `Database` and current auth token.
- `TauriApi`: built resource dispatcher managed by Tauri state.
- `che_api`: Tauri command that dispatches `ApiRequest` actions.
- `InstalledApps`: list of app modules installed into runtime or management CLI.
- `AppModule`: consuming app module trait.
- `ModuleContext`: resource/model registration API used inside modules.
- `ModelSerializer`: model field metadata, payload validation and response serialization.
- `FilterSet`: accepted list filters, local query filtering and remote param mapping.
- `Management`: project-local CLI for consuming apps.
- `TauriSync`: separate managed state for one durable UUID sync database and a supplied transport (`sync` feature).

## Resource Registration

Local SQLite-backed resource:

```rust
ctx.resource::<models::User>(
    "users",
    serializers::user_serializer(),
    filters::user_filterset(),
);
```

Mapped remote resource:

```rust
ctx.mapped_remote_resource::<models::User>(
    "users",
    "/api/users/",
    serializers::user_serializer(),
    filters::user_filterset(),
);
```

Raw remote resource:

```rust
ctx.raw_remote_resource::<models::User>(
    "users",
    "/api/users/",
    serializers::user_serializer(),
    filters::user_filterset(),
);
```

Notes:

- `ctx.resource` creates local table SQL metadata and registers local CRUD handlers.
- `ctx.mapped_remote_resource` normalizes remote JSON through serializers and supports local-only JSON filters.
- `ctx.raw_remote_resource` proxies remote JSON mostly unchanged but still emits generated TS metadata.
- `ctx.remote_resource` currently delegates to mapped remote resource behavior.
- Resource model types require `Model + serde::Serialize + Send + Sync + 'static`.
- Ordinary invoke resources use `#[orm(table = "...")]` and `#[orm(primary_key)]` with i64 IDs. The ORM also supports UUID IDs; sync models use them through separate application CRUD commands.
- Resource serializers use `Model::field_values()` independently of Serde field renames. Timestamp fields are RFC 3339 strings; binary fields are JSON byte arrays.
- Management migrations use compiled `.rs` files with `che_orm::Migration` operations registered through `AppModule::migrations()`; `startapp` generates this hook. Schema state and migration history are managed by the registered `che-orm` migration graph.

## Serializer Rules

- `Field::new("name")` maps frontend field `name` to model/remote source `name`.
- `Field::new("local").source("remote")` maps frontend/local field to remote JSON source.
- `Field::new("id").read_only()` excludes field from create/update payloads.
- `Field::new("password").write_only()` excludes field from responses.
- `Field::new("field").required(false).nullable()` marks optional nullable inputs.
- `Field::json("field")` preserves arbitrary JSON values where supported.
- `Field::related("user", "user_id", &USER_RELATION)` emits nested related response data.
- Serializer metadata drives generated `Response`, `Create`, and `Update` TypeScript interfaces.

## Filter Rules

- Supported lookup constructors: `exact`, `contains`, `gt`, `gte`, `lt`, `lte`.
- Generated query names use Django-style suffixes: `name__contains`, `id__gte`, etc.
- `Filter::exact_source("public", "model_field")` exposes one name and filters another model field.
- `Filter::remote("remote_name")` forwards a different query name to remote APIs.
- `Filter::remote_only("name")` forwards remotely and skips local application.
- `filter.local_only()` applies only after mapped remote JSON normalization.
- `filter.remote_disabled()` keeps local filtering but does not forward to remote APIs.
- `FilterSet::remote_ordering("remote_ordering")` maps frontend `ordering` to a remote ordering parameter.

## Generated TypeScript

`Management::generate_ts` writes:

- `api_client.ts`: generic Tauri invoke API helper.
- `models.ts`: response/create/update/list param interfaces.
- `api.ts`: resource-specific `createModelApi` exports.
- `sync.ts`: `modelSync` plus typed status/conflict/resolution messages, when the management binary enables `sync`.

Do not hand-edit generated frontend files in consuming apps unless the task explicitly asks for it. Change Rust model/serializer/filter/resource metadata and regenerate instead.
For sync client changes, edit `src/sync_client.ts` and regenerate. The Todo UI is hand-written plain JavaScript using invoke directly; it has no generated TypeScript or npm build step.

Default output for Tauri apps is `../src/generated` when run from `src-tauri`.

## Management CLI In Consuming Apps

Typical `src/bin/manage.rs`:

```rust
use che_tauri::Management;

use my_app::apps;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    Management::new(apps::installed_apps()).run().await
}
```

Useful commands from the consuming app backend directory:

```bash
cargo run --bin manage -- startapp users
cargo run --bin manage -- makemigrations users
cargo run --bin manage -- migrate users
cargo run --bin manage -- migrate
cargo run --bin manage -- generate-ts --out ../src/generated
```

## Local Development Commands

Run from `che-tauri`:

```bash
cargo fmt --check
cargo check
cargo test
cargo check --features sync
cargo test --features sync
cargo clippy --all-targets --features sync
```

For changes affecting the Todo example, also run from the library root:

```bash
cargo check --manifest-path examples/todo-sync/Cargo.toml --workspace
cargo test --manifest-path examples/todo-sync/Cargo.toml -p todo-sync-server -p todo-sync-client --no-default-features
cargo clippy --manifest-path examples/todo-sync/Cargo.toml --workspace --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path examples/todo-sync/Cargo.toml --all --check
node --check examples/todo-sync/client/ui/app.js
```

The HTTP tests need ephemeral loopback ports. `--no-default-features` disables the example client's desktop binary and build-script setup, while testing its actual CRUD/runtime library. Use workspace `cargo check` or `cargo build` with default features to verify the desktop binary and exported Tauri command wrappers too.

If changes affect generated API behavior, verify in a consuming app when practical:

```bash
cd ../ewa-mobile/src-tauri
cargo run --bin manage -- generate-ts --out ../src/generated
cd ..
npm run build
```

## Change Guidelines

- Keep changes small and focused.
- Preserve the public API unless the task explicitly requires a breaking change.
- Keep `che-rest` architectural parity in mind before changing module/serializer/filter concepts.
- Update README examples when public API or behavior changes.
- Add or update tests if adding logic that can be tested inside the crate.
- Run `cargo fmt` and `cargo check` before finishing Rust code changes.
- Avoid adding backwards compatibility shims unless there is a concrete consumer need.

## Known Consumers

- `../ewa-mobile/src-tauri` uses `che-tauri` for local/remote resources and generated frontend APIs.

Before making broad API changes, inspect current usage in `../ewa-mobile/src-tauri/src/apps` and generated API expectations in `../ewa-mobile/src/generated`.

## Optional Synchronization

- Enable `sync` for `TauriSync`, `che_model_sync` and generated `sync.ts`.
- `ModuleContext::sync_model` registers protocol-2 client schemas for compiled migrations; it must not emit startup table creation SQL. `InstalledApps::sync_registry` preserves module registration order (FK parents first).
- Sync models have UUID IDs. Existing `che_api` resources retain i64 IDs; application UUID CRUD commands call the managed sync client's `save`, `delete` or configured `database` handle.
- `che-sync` owns persistence, capture, outbox, journals and merge. Supply a `ModelLogTransport`; reuse `che-rest/sync-http` for HTTP rather than duplicating its protocol in Tauri.
- Keep durable sync databases separate from remote response caches. Credential/cache invalidation must not clear pending edits or silently change their server/account. One runtime per database; clones share its locks.
- Run `cargo test --features sync` as well as default tests. HTTP tests need ephemeral loopback ports. The sync tests cover request replay after reopening, conflict resolution, UUID/record input validation and journal pagination.
- Register exported commands with fully qualified paths in `tauri::generate_handler!`, e.g. `che_tauri::che_model_sync` / `che_tauri::che_api`. A bare imported function name may leave Tauri's generated command macros out of scope.
- `AppState` token/config changes affect ordinary remote resources only. The application's sync transport owns its own credentials; renew them for the same server/account without dropping pending data.
- Cache/sync table overlap is currently forbidden by the documented application contract, not a registration-time check. Do not claim that the framework already rejects this configuration. Cache invalidation runs raw deletes and must never include sync tables.
- `pull` downloads one page per model. `flush` sends at most one coalesced record per model, retrying immutable submitted payloads first. Keep UI cycles bounded and stop on `blocked`, conflicts or rejections; no implicit background loop exists.
- Keep revisions and integer protocol values as decimal strings in frontend data. UUIDs are strings. The custom `record` resolution requires an explicit `record` field; null means deletion.

## Todo Example

- `examples/todo-sync/model` shares `Todo` and its ORM `TodoSerializer` between server and transport. This serializer is distinct from `che_tauri::ModelSerializer`, which describes ordinary invoke resources.
- The REST ViewSet uses `IsAuthenticated` and whole-model protocol-2 access. The example has one token-only demo account and binds `127.0.0.1:3007`.
- Server and client `initial.rs` files are frozen compiled migrations. Follow-up model changes require new migration history; never rebuild the initial operations from the current model at runtime.
- Startup does not apply migrations. `todo-sync-server migrate` and `todo-client-db migrate` are explicit setup commands. Preserve repeated-migration behavior.
- Client CRUD goes through the managed `TauriSync`; avoid unconfigured database handles and remote response cache registration.
- `TODO_CLIENT_DB` selects the file for each device. Use separate files for A/B and separate server/account histories. Paths resolve relative to the process working directory.
- The UI uses `window.__TAURI__.core.invoke` with `withGlobalTauri` enabled, local assets and one handler list. Keep the CSP, capability and command names aligned.
- Keep `Cargo.lock` for this application workspace. Ignore generated `client/src-tauri/gen/` schemas, build output and SQLite files; do not commit runtime data.
- When reporting verification, distinguish desktop compilation from actually launching and inspecting the GUI.
