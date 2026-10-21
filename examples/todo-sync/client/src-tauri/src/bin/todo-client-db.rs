use che_orm::Database;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let db = Database::connect(todo_sync_client::database_path()).await?;
    match args.first().map(String::as_str) {
        Some("migrate") => todo_sync_client::migrate(&db).await?,
        Some("migrate-objects") => todo_sync_client::migrate_objects(&db).await?,
        Some("upgrade-objects") => {
            todo_sync_client::upgrade_objects(
                db,
                args.get(1).ok_or("Supply server API base URL")?,
                args.get(2).ok_or("Supply account name")?,
            )
            .await?
        }
        _ => {
            return Err(
                "Usage: todo-client-db [migrate|migrate-objects|upgrade-objects SERVER ACCOUNT]"
                    .into(),
            );
        }
    }
    println!("Client command completed.");
    Ok(())
}
