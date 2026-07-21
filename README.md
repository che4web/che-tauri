# che-tauri

Tauri invoke API layer for `che-orm` applications.

`che-tauri` follows the same app/module/serializer/filter architecture as `che-rest`, but exposes models through a single Tauri command instead of HTTP routes.

## App Modules

```rust
pub mod users;

use che_tauri::InstalledApps;

pub fn installed_apps() -> InstalledApps {
    InstalledApps::new().add(users::module())
}
```

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

For a remote REST-backed resource, use `remote_resource`. The model is used for serializers, filters, and TypeScript metadata, but no local SQLite table is created for that resource.

```rust
ctx.remote_resource::<models::User>(
    "users",
    "/api/users",
    serializers::user_serializer(),
    filters::user_filterset(),
);
```

Remote resources use `[remote].base_url` from `app.toml`:

```toml
[database]
url = "sqlite://app.sqlite?mode=rwc"

[remote]
base_url = "https://api.example.com"
```

REST mapping:

```text
list     GET    /api/users
retrieve GET    /api/users/{id}
create   POST   /api/users
update   PATCH  /api/users/{id}
delete   DELETE /api/users/{id}
```

## Tauri Setup

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

## Frontend Invoke API

Raw invoke request:

```ts
import { invoke } from "@tauri-apps/api/core";

const users = await invoke("che_api", {
  request: {
    resource: "users",
    action: "list",
    params: { name__contains: "Ali" },
  },
});
```

## Management

Project-local `src/bin/manage.rs`:

```rust
use che_tauri::Management;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    Management::new(my_app::apps::installed_apps()).run().await
}
```

Create an app:

```bash
cargo run --bin manage -- startapp users
```

Create migrations:

```bash
cargo run --bin manage -- makemigrations users
```

Apply migrations:

```bash
cargo run --bin manage -- migrate
```

Generate TypeScript models and Tauri invoke client:

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

Generated frontend usage:

```ts
import { userApi } from "./generated/api";

await userApi.create({ name: "Alice" });
const users = await userApi.list({ name__contains: "Ali" });
```
