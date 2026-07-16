use crate::error::AppResult;
use crate::models::{CatalogSkin, SourceRecord};
use std::path::PathBuf;

pub struct SourceStore {
    path: PathBuf,
    sources: Vec<SourceRecord>,
}

impl SourceStore {
    pub fn load(path: PathBuf) -> AppResult<Self> {
        let sources = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path)?)?
        } else {
            Vec::new()
        };
        Ok(Self { path, sources })
    }

    pub fn list(&self) -> Vec<SourceRecord> {
        self.sources.clone()
    }

    pub fn get(&self, source_id: &str) -> Option<SourceRecord> {
        self.sources.iter().find(|item| item.id == source_id).cloned()
    }

    pub fn catalog(&self) -> Vec<CatalogSkin> {
        self.sources
            .iter()
            .flat_map(|source| source.skins.clone())
            .collect()
    }

    pub fn upsert(&mut self, source: SourceRecord) -> AppResult<()> {
        if let Some(existing) = self.sources.iter_mut().find(|item| item.id == source.id) {
            *existing = source;
        } else {
            self.sources.push(source);
        }
        self.persist()
    }

    pub fn remove(&mut self, source_id: &str) -> AppResult<()> {
        self.sources.retain(|source| source.id != source_id);
        self.persist()
    }

    fn persist(&self) -> AppResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&self.sources)?)?;
        std::fs::rename(temporary, &self.path)?;
        Ok(())
    }
}
