//! Optional password-unlocked credential vault integration.
use tauri::Manager;

pub fn install<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    let root = app.path().app_local_data_dir()?;
    std::fs::create_dir_all(&root)?;
    app.plugin(tauri_plugin_stronghold::Builder::with_argon2(&root.join("vault-salt")).build())?;
    Ok(())
}
