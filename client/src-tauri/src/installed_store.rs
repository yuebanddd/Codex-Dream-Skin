use crate::atomic_file::replace_file;
use crate::error::AppResult;
use crate::models::InstalledSkin;
use std::path::PathBuf;

pub struct InstalledStore {
    path: PathBuf,
    skins: Vec<InstalledSkin>,
}

impl InstalledStore {
    pub fn load(path: PathBuf) -> AppResult<Self> {
        let skins = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path)?)?
        } else {
            Vec::new()
        };
        Ok(Self { path, skins })
    }

    pub fn list(&self) -> Vec<InstalledSkin> {
        self.skins.clone()
    }

    pub fn get(&self, source_id: &str, skin_id: &str) -> Option<InstalledSkin> {
        self.skins
            .iter()
            .find(|skin| skin.source_id == source_id && skin.skin_id == skin_id)
            .cloned()
    }

    pub fn upsert(&mut self, skin: InstalledSkin) -> AppResult<()> {
        if let Some(existing) = self
            .skins
            .iter_mut()
            .find(|item| item.source_id == skin.source_id && item.skin_id == skin.skin_id)
        {
            *existing = skin;
        } else {
            self.skins.push(skin);
        }
        self.persist()
    }

    pub fn remove(&mut self, source_id: &str, skin_id: &str) -> AppResult<()> {
        self.skins
            .retain(|skin| skin.source_id != source_id || skin.skin_id != skin_id);
        self.persist()
    }

    fn persist(&self) -> AppResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&self.skins)?)?;
        replace_file(&temporary, &self.path)?;
        Ok(())
    }
}
