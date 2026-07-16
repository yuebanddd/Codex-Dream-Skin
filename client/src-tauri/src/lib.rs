mod commands;
mod error;
mod github_source;
mod install_service;
mod installed_store;
mod models;
mod source_store;

use installed_store::InstalledStore;
use reqwest::Client;
use source_store::SourceStore;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

pub struct AppState {
    http: Client,
    sources: Arc<Mutex<SourceStore>>,
    installed: Arc<Mutex<InstalledStore>>,
    theme_operation: Arc<Mutex<()>>,
    data_dir: PathBuf,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let source_store = SourceStore::load(data_dir.join("sources.json"))?;
            let installed_store = InstalledStore::load(data_dir.join("installed-skins.json"))?;
            let http = Client::builder()
                .user_agent("LumaDrobe/0.2.0")
                .https_only(true)
                .redirect(reqwest::redirect::Policy::limited(3))
                .build()?;
            app.manage(AppState {
                http,
                sources: Arc::new(Mutex::new(source_store)),
                installed: Arc::new(Mutex::new(installed_store)),
                theme_operation: Arc::new(Mutex::new(())),
                data_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_sources,
            commands::list_catalog_skins,
            commands::add_source,
            commands::refresh_source,
            commands::remove_source,
            commands::list_installed_skins,
            commands::install_skin,
            commands::delete_installed_skin,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run LumaDrobe");
}
