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

    pub fn has_storage_collision(&self, source_id: &str, skin_id: &str) -> bool {
        self.skins
            .iter()
            .any(|skin| storage_keys_collide(&skin.source_id, &skin.skin_id, source_id, skin_id))
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

fn storage_keys_collide(
    existing_source_id: &str,
    existing_skin_id: &str,
    source_id: &str,
    skin_id: &str,
) -> bool {
    existing_source_id.eq_ignore_ascii_case(source_id)
        && existing_skin_id.eq_ignore_ascii_case(skin_id)
        && (existing_source_id != source_id || existing_skin_id != skin_id)
}

#[cfg(test)]
mod tests {
    use super::storage_keys_collide;

    #[test]
    fn detects_case_only_storage_collisions() {
        assert!(storage_keys_collide("Foo", "bar", "foo", "bar"));
        assert!(storage_keys_collide("foo", "Bar", "foo", "bar"));
        assert!(!storage_keys_collide("foo", "bar", "foo", "bar"));
        assert!(!storage_keys_collide("foo", "day", "foo", "night"));
    }
}
