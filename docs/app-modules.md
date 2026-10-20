# AppModule: schema, registration and startup

`che-tauri` and `che-rest` use the same common module lifecycle. Their contexts
register different transports: invoke resources in Tauri, routers in REST.
Database schema declaration and database mutation are separate from both.

## Common contract

```rust,ignore
impl che_tauri::AppModule for Notes {
    fn name(&self) -> &'static str { "notes" }

    fn schema(&self) -> che_orm::SchemaSet {
        che_orm::SchemaSet::new().model::<Note>()
    }

    fn migrations(&self) -> Vec<che_orm::Migration> {
        migrations::migrations()
    }

    fn init(&self, ctx: &mut che_tauri::ModuleContext) {
        ctx.resource::<Note>("notes", note_serializer(), note_filterset());
    }

    // Optional hooks, both empty by default:
    fn subscribe(&self, _state: &che_tauri::AppState) {}
    fn start(&self, _state: &che_tauri::AppState) {}
}
```

Modules implement `Send + Sync + 'static`. `schema()` returns pure metadata.
`init()` declaratively registers resources and metadata; TypeScript generation
may call it without running the app. Put runtime subscriptions/workers in the
dedicated hooks, not in `schema()` or `init()`.

`TauriApi::install(apps)` stores the list without calling modules. As with
`Server::install`, another install replaces that list; combine modules through
`InstalledApps::add`. During build, each module's schema and init are collected
in installation order. After successful validation, all subscribe hooks run,
then all start hooks. All registration is complete before any runtime hook.
Build one runtime for the application and manage it in Tauri.

Neither runtime applies migrations or creates tables at startup. Run the app's
explicit migration command first. Registration validation checks metadata; it
does not certify that a database has already applied its compiled history.

## Schema declarations

| Module data | `schema()` declaration |
| --- | --- |
| Local invoke CRUD | `SchemaSet::new().model::<Note>()` |
| Disposable cached remote records | `.model::<CachedRecord>()` |
| Remote-only resources | `SchemaSet::new()` |
| Durable protocol-2 client | `.client_model::<Todo>()` |
| Protocol-2 REST server | `.server_model::<Todo>()` |

Register every persisted local/cache table, including related tables written by
cache normalization. Use dependency order for FKs. Remote-only model metadata
still drives serialization and TypeScript but needs no local table.

`InstalledApps::migration_state()` reads only schemas and includes role-specific
infrastructure. `InstalledApps::sync_registry()` also reads only schemas and
requires client roles. Neither method calls init, subscribes or starts workers.
The registry can therefore be collected independently of API construction.

The Tauri migration generator includes missing shared `che_sync` infrastructure
operations in the selected application's history. Their logical schema owner
remains `che_sync`. Freeze generated operations and register the compiled
history; startup must never compensate for missing migration operations.

## Migrating the earlier che-tauri API

This is an intentional source API change. Update every module implementation:

1. Add required `schema()`. Move table declarations out of init. Ordinary local
   and cached resources no longer add their model to migration state implicitly.
2. Replace `ctx.model::<M>()` / `ctx.create_table::<M>()` with
   `SchemaSet::model::<M>()` in schema. Replace `ctx.sync_model::<M>()` with
   `SchemaSet::client_model::<M>()`. These context methods and context schema
   accessors have been removed.
3. Keep resource registration in init. Schema-only sync modules can have an
   empty init; application UUID commands are registered in the Tauri handler.
4. Remove `TauriApi::initialize()` calls. Apply compiled migrations explicitly
   before build, using the application's management command or reviewed registry.
5. Move side effects out of init into subscribe/start. Install now defers init
   until build. Migrations and sync-registry discovery no longer run init.

`startapp` emits the new schema method. The
[Todo client](../examples/todo-sync/client/src-tauri/src/lib.rs) uses the new
contract with frozen migrations; its existing database layout is unchanged.

For databases with an applied compiled history, moving schema declarations does
not justify editing old operations or checksums. Compare the new desired state
with that history. Databases previously created only by startup DDL need an
application-owned adoption plan: blindly running a new CREATE TABLE baseline
against existing tables will fail. Back up and review their actual schema/data;
there is no implicit baseline/fake-migrate step. A separate disposable cache-only
database can be replaced explicitly, but durable local/sync edits must be kept.

The known `ewa-mobile` consumer still has implementations of the old contract.
It needs explicit schemas for cached models in `projectapp` (Project),
`personemanagment` (Employee), `frcapp` (Frc), and `contractapp` (Contract,
ContractCategory, Counterparty, ContractApplicationFile). Its remote-only
modules can use empty schemas. Review related cached models and existing
migration history there before deployment; that application is outside this
library checkout.

## Registration errors and cache isolation

Build rejects missing local schema tables, duplicate resource names, invalid
schemas, and ordinary local/cached resource tables declared as sync models.
Validation fails before subscribe/start hooks run.

The overlap check covers declared tables in this runtime. It does not inspect
other runtimes, raw SQL or arbitrary database triggers/cascades. Keep disposable
cache databases and their related effects separate from durable sync data.
Cache invalidation must never erase current/shadow/outbox state. Credentials
for `TauriSync` belong to its supplied transport, independently of `AppState`.

## Verification

```bash
cargo fmt --check
cargo check --features sync
cargo test
cargo test --features sync
```

`tests/module_lifecycle.rs` verifies deferred install, build phases, schema-only
discovery, explicit migrations and invalid registrations. Management tests cover
schema-only generation and shared sync tables. Existing CRUD/cache/sync tests
apply explicit test migrations. Also run the separate
[Todo workspace checks](../examples/todo-sync/README.md#проверка) after changes
to this contract. HTTP tests require loopback access; no deployed data is used.
