use std::fs;
use std::path::PathBuf;

use anyhow::Context;

use crate::RoadmapState;
use crate::parser::atomic_write;

#[derive(Debug, Clone)]
pub struct RoadmapStateStore {
    data_dir: PathBuf,
}

impl RoadmapStateStore {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    pub fn path(&self) -> PathBuf {
        self.data_dir.join("roadmaps").join("state.json")
    }

    pub fn load(&self) -> anyhow::Result<Option<RoadmapState>> {
        let path = self.path();
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    pub fn save(&self, state: &RoadmapState) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(state)?;
        atomic_write(&self.path(), &bytes)
    }
}
