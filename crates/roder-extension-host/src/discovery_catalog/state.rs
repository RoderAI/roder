use std::fs;
use std::path::{Path, PathBuf};

use roder_api::discovery::DiscoveryPromotionRecord;

#[derive(Debug, Clone)]
pub struct PromotionStore {
    path: PathBuf,
}

impl PromotionStore {
    pub fn new(session_state_dir: impl AsRef<Path>) -> Self {
        Self {
            path: session_state_dir
                .as_ref()
                .join("discovery")
                .join("promotions.json"),
        }
    }

    pub fn load(&self) -> anyhow::Result<Vec<DiscoveryPromotionRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn save(&self, records: &[DiscoveryPromotionRecord]) -> anyhow::Result<PathBuf> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, serde_json::to_string_pretty(records)?)?;
        Ok(self.path.clone())
    }

    pub(crate) fn ensure(&self) -> anyhow::Result<PathBuf> {
        if self.path.exists() {
            Ok(self.path.clone())
        } else {
            self.save(&[])
        }
    }
}
