# AGENTS.md

## Project Overview

`che-tauri` is a Rust library that exposes `che-orm` model resources to Tauri frontends through a single invoke command.

It mirrors the `che-rest` app/module/serializer/filter architecture, but dispatches model actions through `che_api` instead of HTTP routes. It is used by `ewa-mobile/src-tauri`.

## Stack

- Rust edition 2024.
- Tauri 2 invoke command integration.
- `che-orm` SQLite models and query builders.
- `reqwest` for remote REST resources.
- `serde`/`serde_json` for request and response payloads.
- `clap` for project-local management commands.
- `toml` for app config loading.

## Important Files

- `Cargo.toml`: library dependencies.
- `src/lib.rs`: public exports.
- `src/api.rs`: `TauriApi`, `che_api`, local and remote invoke resources, auth login, generated client template.
- `src/module.rs`: `AppModule`, `InstalledApps`, `ModuleContext`, resource registration metadata.
- `src/serializer.rs`: `Field`, `ModelSerializer`, related serializers, JSON validation and remote normalization.
- `src/filters.rs`: `Filter`, `FilterSet`, lookup parsing, local filters, remote filter forwarding.
- `src/management.rs`: `startapp`, `makemigrations`, `migrate`, `generate-ts`.
- `src/state.rs`: `AppState`, SQLite backend, auth token storage.
- `src/config.rs`: `AppConfig`, `DatabaseConfig`, `RemoteConfig`.
- `src/error.rs`: `ApiError` and `ApiResult`.

## Public Concepts

- `AppState`: config, SQLite backend and current auth token.
- `TauriApi`: built resource dispatcher managed by Tauri state.
- `che_api`: Tauri command that dispatches `ApiRequest` actions.
- `InstalledApps`: list of app modules installed into runtime or management CLI.
- `AppModule`: consuming app module trait.
- `ModuleContext`: resource/model registration API used inside modules.
- `ModelSerializer`: model field metadata, payload validation and response serialization.
- `FilterSet`: accepted list filters, local query filtering and remote param mapping.
- `Management`: project-local CLI for consuming apps.

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
- All resource model types currently require `SqliteModel<Id = i64>`.

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

Do not hand-edit generated frontend files in consuming apps unless the task explicitly asks for it. Change Rust model/serializer/filter/resource metadata and regenerate instead.

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
cargo fmt
cargo check
cargo test
```

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
