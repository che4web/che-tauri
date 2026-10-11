use std::path::{Path, PathBuf};

use che_orm::Database;
use che_tauri::AppState;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "che-tauri-sqlite-url-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn config(&self, database: &Path, mode: &str) -> PathBuf {
        let path = self.0.join(format!("{mode}.toml"));
        let url = format!("sqlite://{}?mode={mode}", database.display());
        std::fs::write(&path, format!("[database]\nurl = {url:?}\n")).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn read_only_config_rejects_writes_and_allows_reads() {
    let temp = TempDir::new();
    let path = temp.0.join("app.sqlite");
    let db = Database::connect(path.to_string_lossy().to_string())
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE probe (id INTEGER)", vec![])
        .await
        .unwrap();
    db.execute_sql("INSERT INTO probe VALUES (1)", vec![])
        .await
        .unwrap();
    drop(db);

    let state = AppState::from_config_file(temp.config(&path, "ro"))
        .await
        .unwrap();
    assert_eq!(
        state
            .db()
            .query_sql("SELECT id FROM probe", vec![])
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        state
            .db()
            .execute_sql("INSERT INTO probe VALUES (2)", vec![])
            .await
            .is_err()
    );
    // A second pool connection must use the same URI/open mode.
    let (first, second) = tokio::join!(
        state.db().query_sql("SELECT id FROM probe", vec![]),
        state.db().execute_sql("DELETE FROM probe", vec![]),
    );
    assert_eq!(first.unwrap().len(), 1);
    assert!(second.is_err());
}

#[tokio::test]
async fn read_write_mode_does_not_create_files_but_rwc_does() {
    let temp = TempDir::new();
    let path = temp.0.join("missing.sqlite");
    assert!(
        AppState::from_config_file(temp.config(&path, "rw"))
            .await
            .is_err()
    );
    assert!(!path.exists());
    let state = AppState::from_config_file(temp.config(&path, "rwc"))
        .await
        .unwrap();
    state
        .db()
        .execute_sql("CREATE TABLE probe (id INTEGER)", vec![])
        .await
        .unwrap();
    assert!(path.exists());
}

#[tokio::test]
async fn in_memory_uris_share_one_database_across_operations() {
    let temp = TempDir::new();
    for url in [
        ":memory:",
        "sqlite://:memory:?cache=shared",
        "sqlite://che-tauri-url-test?mode=memory&cache=shared",
    ] {
        let config = temp.0.join("memory.toml");
        std::fs::write(&config, format!("[database]\nurl = {url:?}\n")).unwrap();
        let state = AppState::from_config_file(config).await.unwrap();
        state
            .db()
            .execute_sql("CREATE TABLE probe (id INTEGER)", vec![])
            .await
            .unwrap();
        let (first, second) = tokio::join!(
            state
                .db()
                .execute_sql("INSERT INTO probe VALUES (1)", vec![]),
            state
                .db()
                .execute_sql("INSERT INTO probe VALUES (2)", vec![]),
        );
        first.unwrap();
        second.unwrap();
        assert_eq!(
            state
                .db()
                .query_sql("SELECT id FROM probe", vec![])
                .await
                .unwrap()
                .len(),
            2
        );
    }
}
