use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub author: String,
    #[serde(default)]
    pub description: Option<String>,
    pub skins: Vec<SourceSkinEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSkinEntry {
    pub id: String,
    pub manifest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    pub author: String,
    #[serde(default)]
    pub engine_version: Option<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub preview: Option<String>,
    pub background: String,
    #[serde(default)]
    pub css: Option<String>,
    #[serde(default)]
    pub colors: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSkin {
    pub source_id: String,
    pub source_name: String,
    pub manifest_path: String,
    #[serde(default)]
    pub preview_url: Option<String>,
    pub background_url: String,
    #[serde(default)]
    pub css_url: Option<String>,
    pub manifest: SkinManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSkin {
    pub source_id: String,
    pub source_name: String,
    pub skin_id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub installed_at: String,
    pub install_dir: String,
    pub background_path: String,
    #[serde(default)]
    pub preview_path: Option<String>,
    #[serde(default)]
    pub css_path: Option<String>,
    #[serde(default)]
    pub asset_hashes: BTreeMap<String, String>,
    pub manifest: SkinManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub phase: String,
    #[serde(default)]
    pub active_source_id: Option<String>,
    #[serde(default)]
    pub active_skin_id: Option<String>,
    #[serde(default)]
    pub active_version: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiagnostics {
    pub generated_at: String,
    pub client_version: String,
    pub platform: String,
    pub architecture: String,
    pub runtime: RuntimeStatus,
    pub saved_session: bool,
    pub paused: bool,
    pub codex_found: bool,
    #[serde(default)]
    pub codex_version: Option<String>,
    #[serde(default)]
    pub codex_identity: Option<String>,
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub codex_running: Option<bool>,
    #[serde(default)]
    pub listener_verified: Option<bool>,
    #[serde(default)]
    pub endpoint_verified: Option<bool>,
    #[serde(default)]
    pub verified_targets: Option<usize>,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl RuntimeStatus {
    pub fn stopped(message: impl Into<String>) -> Self {
        Self {
            phase: "stopped".into(),
            active_source_id: None,
            active_skin_id: None,
            active_version: None,
            port: None,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecord {
    pub id: String,
    pub repository_url: String,
    pub owner: String,
    pub repository: String,
    pub ref_name: String,
    pub name: String,
    pub author: String,
    #[serde(default)]
    pub description: Option<String>,
    pub refreshed_at: String,
    pub skins: Vec<CatalogSkin>,
}

#[derive(Debug, Deserialize)]
pub struct GithubRepository {
    pub default_branch: String,
}
