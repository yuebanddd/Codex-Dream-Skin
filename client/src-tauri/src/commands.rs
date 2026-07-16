use crate::github_source::fetch_source;
use crate::install_service::{install_skin as install_theme, remove_installed_skin};
use crate::models::{CatalogSkin, InstalledSkin, SourceRecord};
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn list_sources(state: State<'_, AppState>) -> Result<Vec<SourceRecord>, String> {
    Ok(state.sources.lock().await.list())
}

#[tauri::command]
pub async fn list_catalog_skins(state: State<'_, AppState>) -> Result<Vec<CatalogSkin>, String> {
    Ok(state.sources.lock().await.catalog())
}

#[tauri::command]
pub async fn add_source(
    repository_url: String,
    state: State<'_, AppState>,
) -> Result<SourceRecord, String> {
    let source = fetch_source(&state.http, &repository_url)
        .await
        .map_err(|error| error.to_string())?;
    state
        .sources
        .lock()
        .await
        .upsert(source.clone())
        .map_err(|error| error.to_string())?;
    Ok(source)
}

#[tauri::command]
pub async fn refresh_source(
    source_id: String,
    state: State<'_, AppState>,
) -> Result<SourceRecord, String> {
    let existing = state
        .sources
        .lock()
        .await
        .get(&source_id)
        .ok_or_else(|| format!("找不到订阅源：{source_id}"))?;
    let repository_url = format!("{}?ref={}", existing.repository_url, existing.ref_name);
    let source = fetch_source(&state.http, &repository_url)
        .await
        .map_err(|error| error.to_string())?;
    state
        .sources
        .lock()
        .await
        .upsert(source.clone())
        .map_err(|error| error.to_string())?;
    Ok(source)
}

#[tauri::command]
pub async fn remove_source(source_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state
        .sources
        .lock()
        .await
        .remove(&source_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_installed_skins(
    state: State<'_, AppState>,
) -> Result<Vec<InstalledSkin>, String> {
    Ok(state.installed.lock().await.list())
}

#[tauri::command]
pub async fn install_skin(
    source_id: String,
    skin_id: String,
    state: State<'_, AppState>,
) -> Result<InstalledSkin, String> {
    let _operation = state.theme_operation.lock().await;
    let previous = state.installed.lock().await.get(&source_id, &skin_id);
    let skin = state
        .sources
        .lock()
        .await
        .catalog()
        .into_iter()
        .find(|skin| skin.source_id == source_id && skin.manifest.id == skin_id)
        .ok_or_else(|| format!("找不到主题：{source_id}/{skin_id}"))?;
    let installed = install_theme(&state.http, &state.data_dir, &skin)
        .await
        .map_err(|error| error.to_string())?;
    state
        .installed
        .lock()
        .await
        .upsert(installed.clone())
        .map_err(|error| error.to_string())?;
    if let Some(previous) = previous.filter(|item| item.version != installed.version) {
        remove_installed_skin(
            &state.data_dir,
            &previous.source_id,
            &previous.skin_id,
            &previous.version,
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(installed)
}

#[tauri::command]
pub async fn delete_installed_skin(
    source_id: String,
    skin_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _operation = state.theme_operation.lock().await;
    let installed = state
        .installed
        .lock()
        .await
        .get(&source_id, &skin_id)
        .ok_or_else(|| format!("主题尚未安装：{source_id}/{skin_id}"))?;
    remove_installed_skin(
        &state.data_dir,
        &installed.source_id,
        &installed.skin_id,
        &installed.version,
    )
    .map_err(|error| error.to_string())?;
    state
        .installed
        .lock()
        .await
        .remove(&source_id, &skin_id)
        .map_err(|error| error.to_string())
}
