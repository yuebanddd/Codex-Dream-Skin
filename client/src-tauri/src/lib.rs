mod commands;
mod error;
mod github_source;
mod models;
mod source_store;

use reqwest::Client;
use source_store::SourceStore;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

pub struct AppState {
    http: Client,
    sources: Arc<Mutex<SourceStore>>,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let source_store = SourceStore::load(data_dir.join("sources.json"))?;
            let http = Client::builder()
                .user_agent("Codex-Dream-Skin/0.1.0")
                .https_only(true)
                .build()?;
            app.manage(AppState {
                http,
                sources: Arc::new(Mutex::new(source_store)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_sources,
            commands::list_catalog_skins,
            commands::add_source,
            commands::refresh_source,
            commands::remove_source,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Codex Dream Skin");
}
