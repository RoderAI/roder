use std::path::{Path, PathBuf};

use anyhow::Context;
use roder_api::memory::MemoryScope;
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod markdown;

use markdown::load_markdown_corpus_dir;

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

pub(crate) struct LoadedImportBatch {
    pub source_path: Option<String>,
    pub source_hash: String,
    pub records: Vec<LoadedImportRecord>,
    pub errors: usize,
}

pub(crate) struct LoadedImportRecord {
    pub line_index: usize,
    pub record: JsonlImportRecord,
}

pub(crate) fn load_import_batch(
    input: &ImportBatchInput,
    format: &str,
) -> anyhow::Result<LoadedImportBatch> {
    match input {
        ImportBatchInput::Path(path) if path.is_dir() => load_markdown_corpus_dir(path, format),
        ImportBatchInput::Path(path) => load_jsonl_path(path, format),
        ImportBatchInput::JsonlString(payload) => load_jsonl_string(payload, format),
    }
}

fn load_jsonl_path(path: &Path, format: &str) -> anyhow::Result<LoadedImportBatch> {
    ensure_jsonl_format(format)?;
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("read import input {}", path.display()))?;
    let mut loaded = load_jsonl_body(&body)?;
    loaded.source_path = Some(path.display().to_string());
    Ok(loaded)
}

fn load_jsonl_string(payload: &str, format: &str) -> anyhow::Result<LoadedImportBatch> {
    ensure_jsonl_format(format)?;
    load_jsonl_body(payload)
}

fn load_jsonl_body(body: &str) -> anyhow::Result<LoadedImportBatch> {
    let mut records = Vec::new();
    let mut errors = 0usize;
    for (line_index, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(record) => records.push(LoadedImportRecord { line_index, record }),
            Err(_) => errors += 1,
        }
    }
    Ok(LoadedImportBatch {
        source_path: None,
        source_hash: crate::model::content_hash(body),
        records,
        errors,
    })
}

fn ensure_jsonl_format(format: &str) -> anyhow::Result<()> {
    if format == "jsonl" {
        return Ok(());
    }
    anyhow::bail!("unsupported import format {format:?}; expected jsonl")
}

fn ensure_directory_format(format: &str) -> anyhow::Result<()> {
    match format {
        "directory" | "markdown" | "markdown_dir" | "markdown-dir" | "corpus" | "jsonl" => Ok(()),
        other => anyhow::bail!(
            "unsupported directory import format {other:?}; expected directory|markdown|corpus"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ImportBatchInput, load_import_batch};

    #[test]
    fn loads_markdown_corpus_directory() {
        let root =
            std::env::temp_dir().join(format!("roder-gbrain-import-test-{}", uuid::Uuid::new_v4()));
        let event_dir = root.join("EV-1");
        fs::create_dir_all(&event_dir).unwrap();
        fs::write(
            event_dir.join("ART-1.md"),
            "<!-- artefact_metadata\nslot_id: ART-1\nevent_id: EV-1\ngenre: meeting_notes\nrole: primary\nauthor: P-1\ntestimony_type: direct\n-->\n\n2026-06-07\n\nMaya approved the plan.\n",
        )
        .unwrap();

        let loaded = load_import_batch(&ImportBatchInput::Path(root.clone()), "directory").unwrap();

        assert_eq!(loaded.records.len(), 1);
        assert_eq!(loaded.errors, 0);
        let record = &loaded.records[0].record;
        assert_eq!(record.slug.as_deref(), Some("ART-1"));
        assert_eq!(record.thread_id.as_deref(), Some("EV-1"));
        assert_eq!(record.valid_at.as_deref(), Some("2026-06-07"));
        assert!(record.text.contains("Maya approved the plan."));
        assert_eq!(
            record
                .metadata
                .get("source_type")
                .and_then(|value| value.as_str()),
            Some("meeting_notes")
        );
        assert!(record.provenance.iter().any(|item| item == "ART-1"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loads_one_line_markdown_corpus_headers() {
        let root = std::env::temp_dir().join(format!(
            "roder-gbrain-header-import-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(root.join("_atmosphere")).unwrap();
        fs::create_dir_all(root.join("_emergent").join("P02")).unwrap();
        fs::write(
            root.join("_atmosphere")
                .join("ATM-slack-finance-202106.md"),
            "<!-- atmosphere slack channel=#finance 2021-06 role=noise -->\n\n[2021-06-02 09:12] @Maya: Finance roadmap posted.\n",
        )
        .unwrap();
        fs::write(
            root.join("_emergent").join("P02").join("EMG-P02-F1-1.md"),
            "<!-- emergent_evidence pattern=P02 facet=1 2023-10-18 role=emergent_evidence -->\n\n[email_thread] 2023-10-18\n\nDaniel asked Arjun to lead the data privacy patch.\n",
        )
        .unwrap();

        let loaded = load_import_batch(&ImportBatchInput::Path(root.clone()), "directory").unwrap();

        assert_eq!(loaded.records.len(), 2);
        assert_eq!(loaded.errors, 0);
        let atmosphere = loaded
            .records
            .iter()
            .map(|loaded| &loaded.record)
            .find(|record| {
                record
                    .slug
                    .as_deref()
                    .is_some_and(|id| id == "ATM-slack-finance-202106")
            })
            .unwrap();
        assert_eq!(
            atmosphere.thread_id.as_deref(),
            Some("ATM-slack-finance-202106")
        );
        assert_eq!(
            atmosphere
                .metadata
                .get("source_type")
                .and_then(|value| value.as_str()),
            Some("atmosphere")
        );
        assert_eq!(
            atmosphere
                .metadata
                .get("channel")
                .and_then(|value| value.as_str()),
            Some("#finance")
        );

        let emergent = loaded
            .records
            .iter()
            .map(|loaded| &loaded.record)
            .find(|record| {
                record
                    .slug
                    .as_deref()
                    .is_some_and(|id| id == "EMG-P02-F1-1")
            })
            .unwrap();
        assert_eq!(emergent.thread_id.as_deref(), Some("P02"));
        assert_eq!(emergent.valid_at.as_deref(), Some("2023-10-18"));
        assert_eq!(
            emergent
                .metadata
                .get("source_type")
                .and_then(|value| value.as_str()),
            Some("emergent_evidence")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loads_gbrain_style_markdown_directory_with_frontmatter_and_pruning() {
        let root = std::env::temp_dir().join(format!(
            "roder-gbrain-markdown-import-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(root.join("people")).unwrap();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::create_dir_all(root.join("node_modules").join("pkg")).unwrap();
        fs::write(
            root.join("people").join("Maya Patel.md"),
            "---\ntype: person\ntitle: Maya Patel\ndate: 2022-05-01\ntags: [helix]\n---\n\nMaya approved the 90-day retention policy.\n\n<!-- timeline -->\n2022-05-01\n",
        )
        .unwrap();
        fs::write(
            root.join("people").join("Daniel Ortiz.mdx"),
            "# Daniel Ortiz\n\nDaniel supported GraphQL on 2022-07-20.\n",
        )
        .unwrap();
        fs::write(root.join("README.md"), "not a brain page").unwrap();
        fs::write(root.join(".obsidian").join("Hidden.md"), "hidden").unwrap();
        fs::write(
            root.join("node_modules").join("pkg").join("Package.md"),
            "vendor",
        )
        .unwrap();

        let loaded = load_import_batch(&ImportBatchInput::Path(root.clone()), "directory").unwrap();

        assert_eq!(loaded.records.len(), 2);
        assert_eq!(loaded.errors, 0);
        let maya = loaded
            .records
            .iter()
            .map(|loaded| &loaded.record)
            .find(|record| {
                record
                    .slug
                    .as_deref()
                    .is_some_and(|slug| slug == "people/maya-patel")
            })
            .unwrap();
        assert_eq!(maya.subject.as_deref(), Some("Maya Patel"));
        assert_eq!(maya.valid_at.as_deref(), Some("2022-05-01"));
        assert_eq!(
            maya.metadata
                .get("source_type")
                .and_then(|value| value.as_str()),
            Some("person")
        );
        assert_eq!(
            maya.metadata
                .get("source_path")
                .and_then(|value| value.as_str()),
            Some("people/Maya Patel.md")
        );
        assert!(maya.text.contains("Maya approved"));
        assert!(!maya.text.contains("type: person"));

        let daniel = loaded
            .records
            .iter()
            .map(|loaded| &loaded.record)
            .find(|record| {
                record
                    .slug
                    .as_deref()
                    .is_some_and(|slug| slug == "people/daniel-ortiz")
            })
            .unwrap();
        assert_eq!(daniel.subject.as_deref(), Some("Daniel Ortiz"));
        assert_eq!(daniel.valid_at.as_deref(), Some("2022-07-20"));
        assert_eq!(
            daniel
                .metadata
                .get("source_type")
                .and_then(|value| value.as_str()),
            Some("person")
        );

        fs::remove_dir_all(root).unwrap();
    }
}
