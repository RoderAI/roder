use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct LoadedFileBackedFixture {
    pub dir: PathBuf,
    pub fixture: FileBackedContextFixture,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileBackedContextSuite {
    pub name: String,
    pub version: u32,
    #[serde(default)]
    pub description: Option<String>,
    pub fixtures: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileBackedContextFixture {
    pub id: String,
    pub suite: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub prompt: String,
    pub expected_answer: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    pub grading: FileBackedGradingRules,
    pub context: FileBackedContextFiles,
    #[serde(default)]
    pub metrics: FileBackedMetricFlags,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileBackedGradingRules {
    #[serde(default)]
    pub require_artifact_follow_up: bool,
    #[serde(default)]
    pub secret_line: Option<u64>,
    #[serde(default)]
    pub secret_pattern: Option<String>,
    #[serde(default)]
    pub secret_must_not_appear_in_inline: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileBackedContextFiles {
    pub inline_path: String,
    #[serde(default)]
    pub full_output_path: Option<String>,
    pub artifacts: Vec<FileBackedArtifactRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileBackedArtifactRef {
    pub artifact_id: String,
    pub kind: String,
    pub label: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FileBackedMetricFlags {
    #[serde(default)]
    pub track_inline_chars_saved: bool,
    #[serde(default)]
    pub track_artifact_reads: bool,
    #[serde(default)]
    pub track_artifact_grep_calls: bool,
    #[serde(default)]
    pub track_answer_correctness: bool,
}

fn default_timeout_secs() -> u64 {
    120
}

pub fn default_file_backed_suite_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals/fixtures/context/file-backed")
}

pub fn load_file_backed_suite(
    dir: impl AsRef<Path>,
) -> anyhow::Result<(FileBackedContextSuite, Vec<LoadedFileBackedFixture>)> {
    let dir = dir.as_ref();
    let suite: FileBackedContextSuite =
        serde_json::from_str(&std::fs::read_to_string(dir.join("suite.json"))?)?;
    let mut fixtures = Vec::with_capacity(suite.fixtures.len());
    for relative in &suite.fixtures {
        let fixture_dir = dir.join(Path::new(relative).parent().unwrap_or(Path::new(".")));
        let fixture: FileBackedContextFixture =
            serde_json::from_str(&std::fs::read_to_string(dir.join(relative))?)?;
        fixtures.push(LoadedFileBackedFixture {
            dir: fixture_dir,
            fixture,
        });
    }
    Ok((suite, fixtures))
}

impl LoadedFileBackedFixture {
    pub fn read_inline(&self) -> anyhow::Result<String> {
        Ok(std::fs::read_to_string(
            self.dir.join(&self.fixture.context.inline_path),
        )?)
    }

    pub fn read_artifact(&self, artifact_id: &str) -> anyhow::Result<String> {
        let artifact = self
            .fixture
            .context
            .artifacts
            .iter()
            .find(|a| a.artifact_id == artifact_id)
            .ok_or_else(|| anyhow::anyhow!("unknown artifact id {artifact_id}"))?;
        Ok(std::fs::read_to_string(self.dir.join(&artifact.path))?)
    }

    pub fn read_full_output(&self) -> anyhow::Result<Option<String>> {
        let Some(path) = &self.fixture.context.full_output_path else {
            return Ok(None);
        };
        Ok(Some(std::fs::read_to_string(self.dir.join(path))?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backed_context_suite_round_trips() {
        let dir = default_file_backed_suite_dir();
        let (suite, fixtures) = load_file_backed_suite(&dir).unwrap();
        assert_eq!(suite.name, "file-backed-context");
        assert_eq!(fixtures.len(), 2);
        assert!(fixtures
            .iter()
            .any(|f| f.fixture.id == "long-command-secret-line"));
        assert!(fixtures
            .iter()
            .any(|f| f.fixture.id == "compaction-history-recovery"));
    }
}
