use che_orm::Database;
use todo_sync_server::{DEMO_TOKEN, apps, migrate, state};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var("TODO_SERVER_DB").unwrap_or_else(|_| "server.sqlite".into());
    let db = Database::connect(path).await?;
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str).unwrap_or("serve") {
        "migrate" => {
            migrate(&db).await?;
            println!("Server migrations applied.");
        }
        "migrate-objects"=>{todo_sync_server::shared::migrate(&db).await?;println!("Object-delivery migrations applied.");}
        "serve-objects"=>{
            let router=che_rest::Server::new(todo_sync_server::shared::state(db).await?).install(todo_sync_server::shared::apps()).build().await?;
            let listener=tokio::net::TcpListener::bind("127.0.0.1:3007").await?;
            println!("Shared Todo API: alice=todo-alice, bob=todo-bob (demo tokens)");
            axum::serve(listener,router).await?;
        }
        "grant"|"revoke"=>{
            let id=args.get(1).ok_or("Supply object UUID")?.parse()?;
            let username=args.get(2).ok_or("Supply target username")?;
            todo_sync_server::shared::share(&db,id,username,args[0]=="grant").await?;
            println!("Object access updated.");
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
        _ => return Err("Usage: todo-sync-server [migrate|serve|migrate-objects|serve-objects|grant UUID USER|revoke UUID USER]".into()),
    }
    Ok(())
}
