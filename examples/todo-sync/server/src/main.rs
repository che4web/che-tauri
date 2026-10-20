use che_orm::Database;
use todo_sync_server::{DEMO_TOKEN, apps, migrate, state};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var("TODO_SERVER_DB").unwrap_or_else(|_| "server.sqlite".into());
    let db = Database::connect(path).await?;
    match std::env::args().nth(1).as_deref().unwrap_or("serve") {
        "migrate" => {
            migrate(&db).await?;
            println!("Server migrations applied.");
        }
        "serve" => {
            let token = std::env::var("TODO_SYNC_TOKEN").unwrap_or_else(|_| DEMO_TOKEN.into());
            let router = che_rest::Server::new(state(db, &token).await?)
                .install(apps())
                .build()
                .await?;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:3007").await?;
            println!("Todo API: http://127.0.0.1:3007/api/todos/ (demo token authentication)");
            axum::serve(listener, router).await?;
        }
        _ => return Err("Usage: todo-sync-server [migrate|serve]".into()),
    }
    Ok(())
}
