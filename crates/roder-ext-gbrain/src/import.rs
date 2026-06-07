use std::path::PathBuf;

use roder_api::memory::MemoryScope;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DedupeMode {
    SourceId,
    ContentHash,
    Both,
}

impl DedupeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceId => "source_id",
            Self::ContentHash => "content_hash",
            Self::Both => "both",
        }
    }
}

impl std::str::FromStr for DedupeMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "source_id" | "source-id" | "sourceId" => Ok(Self::SourceId),
            "content_hash" | "content-hash" | "contentHash" => Ok(Self::ContentHash),
            "both" => Ok(Self::Both),
            other => {
                anyhow::bail!("unknown dedupe mode {other:?}; expected source_id|content_hash|both")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum ImportBatchInput {
    Path(PathBuf),
    JsonlString(String),
}

#[derive(Debug, Clone)]
pub struct ImportBatchParams {
    pub input: ImportBatchInput,
    pub format: String,
    pub scope: MemoryScope,
    pub source: Option<String>,
    pub dedupe: DedupeMode,
    pub dream_after_import: Option<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportBatchResult {
    pub run_id: String,
    pub status: String,
    pub scope_id: String,
    pub source: Option<String>,
    pub format: String,
    pub dedupe: DedupeMode,
    pub total: usize,
    pub inserted: usize,
    pub skipped_duplicates: usize,
    pub errors: usize,
    pub fact_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonlImportRecord {
    pub text: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub valid_at: Option<String>,
    #[serde(default)]
    pub invalid_at: Option<String>,
    #[serde(default)]
    pub ingested_at: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub provenance: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}
