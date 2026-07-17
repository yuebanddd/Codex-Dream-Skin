mod atomic_file;
mod cdp;
mod codex_process;
mod commands;
mod error;
mod github_source;
mod install_service;
mod installed_store;
mod models;
mod renderer_payload;
mod runtime_log;
mod runtime_service;
mod source_store;

use installed_store::InstalledStore;
use reqwest::Client;
use runtime_service::RuntimeManager;
use source_store::SourceStore;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

pub struct AppState {
    http: Client,
    sources: Arc<Mutex<SourceStore>>,
    installed: Arc<Mutex<InstalledStore>>,
    runtime: Arc<RuntimeManager>,
    theme_operation: Arc<Mutex<()>>,
    data_dir: PathBuf,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let source_store = SourceStore::load(data_dir.join("sources.json"))?;
            let installed_store = Arc::new(Mutex::new(InstalledStore::load(
                data_dir.join("installed-skins.json"),
            )?));
            let http = Client::builder()
                .user_agent("LumaDrobe/0.5.1")
                .https_only(true)
                .redirect(reqwest::redirect::Policy::limited(3))
                .build()?;
            let runtime = RuntimeManager::new(data_dir.join("runtime.json"))?;
            let theme_operation = Arc::new(Mutex::new(()));
            let recovery_runtime = Arc::clone(&runtime);
            let recovery_installed = Arc::clone(&installed_store);
            let recovery_operation = Arc::clone(&theme_operation);
            tauri::async_runtime::spawn(async move {
                let _operation = recovery_operation.lock().await;
                recovery_runtime.recover(recovery_installed).await;
            });
            app.manage(AppState {
                http,
                sources: Arc::new(Mutex::new(source_store)),
                installed: installed_store,
                runtime,
                theme_operation,
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
            commands::runtime_status,
            commands::runtime_diagnostics,
            commands::export_runtime_diagnostics,
            commands::apply_and_launch,
            commands::pause_theme,
            commands::resume_theme,
            commands::restore_native,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run LumaDrobe");
}
