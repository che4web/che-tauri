use che_orm::Database;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() != Some("migrate") {
        return Err("Usage: todo-client-db migrate".into());
    }
    let db = Database::connect(todo_sync_client::database_path()).await?;
    todo_sync_client::migrate(&db).await?;
    println!("Client migrations applied.");
    Ok(())
}
