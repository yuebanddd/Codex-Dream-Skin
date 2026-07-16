use crate::github_source::fetch_source;
use crate::models::{CatalogSkin, SourceRecord};
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
