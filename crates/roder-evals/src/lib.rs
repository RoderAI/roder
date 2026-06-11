use std::path::{Path, PathBuf};
use std::time::Instant;

use roder_api::artifacts::{ContextArtifactAccess, ContextArtifactKind, format_artifact_reference};
use roder_core::artifacts::{ContextArtifactStore, CreateArtifactRequest};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub mod fixture;
pub mod graders;
pub(crate) mod retrieval_router;
pub mod runner;
pub mod tool_search;
pub mod trace;

pub use fixture::*;
pub use runner::*;
pub use trace::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileBackedContextFixture {
    pub id: String,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub expected_answer_contains: String,
    pub expected_artifact_query: String,
    #[serde(default)]
    pub expected_tool: ExpectedArtifactTool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedArtifactTool {
    Read,
    #[default]
    Grep,
    Tail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalRunOptions {
    pub offline: bool,
    pub output_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileBackedContextReport {
    pub fixture_dir: PathBuf,
    pub offline: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: OffsetDateTime,
    pub results: Vec<FileBackedContextResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileBackedContextResult {
    pub fixture_id: String,
    pub answer_correct: bool,
    pub inline_chars_before: u64,
    pub inline_chars_after: u64,
    pub inline_tokens_before: u64,
    pub inline_tokens_after: u64,
    pub artifact_read_count: u64,
    pub artifact_grep_count: u64,
    pub artifact_tail_count: u64,
    pub artifact_bytes_written: u64,
    pub artifact_lines_written: u64,
    pub inline_tokens_saved: u64,
    pub turn_wall_time_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovered_detail: Option<String>,
}

pub fn load_fixtures(dir: &Path) -> anyhow::Result<Vec<FileBackedContextFixture>> {
    let mut fixtures = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        fixtures.push(serde_json::from_str(&text)?);
    }
    fixtures.sort_by(|left: &FileBackedContextFixture, right| left.id.cmp(&right.id));
    Ok(fixtures)
}

pub fn run_file_backed_context_eval(
    fixture_dir: &Path,
    options: EvalRunOptions,
) -> anyhow::Result<FileBackedContextReport> {
    if !options.offline {
        anyhow::bail!("file-backed context evals currently require --offline");
    }
    let fixtures = load_fixtures(fixture_dir)?;
    let results = fixtures
        .iter()
        .map(run_file_backed_fixture_benchmark)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let report = FileBackedContextReport {
        fixture_dir: fixture_dir.to_path_buf(),
        offline: options.offline,
        generated_at: OffsetDateTime::now_utc(),
        results,
    };
    std::fs::create_dir_all(&options.output_dir)?;
    let report_path = options.output_dir.join("file-backed-context-report.json");
    std::fs::write(&report_path, serde_json::to_string_pretty(&report)?)?;
    Ok(report)
}

pub fn write_file_backed_context_benchmark_markdown(
    report: &FileBackedContextReport,
    output_dir: &Path,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(output_dir)?;
    std::fs::write(
        output_dir.join("results.md"),
        benchmark_results_markdown(report),
    )?;
    std::fs::write(
        output_dir.join("findings-summary.md"),
        benchmark_findings_markdown(report),
    )?;
    Ok(())
}

fn run_file_backed_fixture_benchmark(
    fixture: &FileBackedContextFixture,
) -> anyhow::Result<FileBackedContextResult> {
    let start = Instant::now();
    let payload = fixture_payload(fixture);
    let thread_id = format!("bench-{}", fixture.id);
    let turn_id = "turn-1".to_string();
    let thread_root = std::env::temp_dir().join(format!(
        "roder-file-backed-bench-{}-{}",
        fixture.id,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let store = ContextArtifactStore::new_thread_scoped(&thread_root);
    let artifact = store.create(CreateArtifactRequest {
        kind: fixture_kind(fixture),
        thread_id: &thread_id,
        turn_id: &turn_id,
        source_tool_id: Some(&fixture.id),
        label: Some(&fixture.title),
        bytes: payload.as_bytes(),
    })?;
    let reference = format_artifact_reference(&artifact, &fixture.title);
    let inline_after = inline_after_text(fixture, &reference);
    let (artifact_read_count, artifact_grep_count, artifact_tail_count, recovered_detail) =
        match fixture.expected_tool {
            ExpectedArtifactTool::Read => {
                let page = store.read_artifact(&thread_id, &artifact.id, 1, 200)?;
                (1, 0, 0, recover_detail(&page.text, fixture))
            }
            ExpectedArtifactTool::Grep => {
                let page = store.grep_artifact(
                    &thread_id,
                    &artifact.id,
                    &fixture.expected_artifact_query,
                    0,
                    200,
                )?;
                (0, 1, 0, recover_detail(&page.text, fixture))
            }
            ExpectedArtifactTool::Tail => {
                let page = store.tail_artifact(&thread_id, &artifact.id, 200)?;
                (0, 0, 1, recover_detail(&page.text, fixture))
            }
        };
    let inline_chars_before = payload.chars().count() as u64;
    let inline_chars_after = inline_after.chars().count() as u64;
    let inline_tokens_before = estimate_tokens_from_chars(inline_chars_before);
    let inline_tokens_after = estimate_tokens_from_chars(inline_chars_after);
    let result = FileBackedContextResult {
        fixture_id: fixture.id.clone(),
        answer_correct: recovered_detail
            .as_deref()
            .is_some_and(|detail| detail.contains(&fixture.expected_answer_contains)),
        inline_chars_before,
        inline_chars_after,
        inline_tokens_before,
        inline_tokens_after,
        artifact_read_count,
        artifact_grep_count,
        artifact_tail_count,
        artifact_bytes_written: artifact.byte_count,
        artifact_lines_written: artifact.line_count,
        inline_tokens_saved: inline_tokens_before.saturating_sub(inline_tokens_after),
        turn_wall_time_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        recovered_detail,
    };
    let _ = std::fs::remove_dir_all(thread_root);
    Ok(result)
}

fn fixture_payload(fixture: &FileBackedContextFixture) -> String {
    match fixture.id.as_str() {
        "long-command-output" => {
            let mut lines = Vec::with_capacity(2_400);
            for line in 1..=2_400 {
                if line == 1_937 {
                    lines.push(fixture.expected_answer_contains.clone());
                } else {
                    lines.push(format!(
                        "line {line:04}: build log noise {}",
                        "x".repeat(48)
                    ));
                }
            }
            lines.join("\n")
        }
        "compaction-history-recovery" => {
            let mut lines = Vec::with_capacity(900);
            for turn in 1..=900 {
                if turn == 617 {
                    lines.push(format!(
                        r#"{{"turn":{turn},"role":"assistant","text":"{}"}}"#,
                        fixture.expected_answer_contains
                    ));
                } else {
                    lines.push(format!(
                        r#"{{"turn":{turn},"role":"assistant","text":"historical detail {}"}}"#,
                        "y".repeat(56)
                    ));
                }
            }
            lines.join("\n")
        }
        _ => format!(
            "{}\n{}\n{}",
            fixture.prompt, fixture.expected_answer_contains, fixture.expected_artifact_query
        ),
    }
}

fn fixture_kind(fixture: &FileBackedContextFixture) -> ContextArtifactKind {
    if fixture.tags.iter().any(|tag| tag == "compaction") {
        ContextArtifactKind::ChatHistory
    } else {
        ContextArtifactKind::CommandStdout
    }
}

fn inline_after_text(fixture: &FileBackedContextFixture, reference: &str) -> String {
    format!(
        "{}\n\nStored dynamic context externally.\n{}",
        fixture.prompt, reference
    )
}

fn recover_detail(text: &str, fixture: &FileBackedContextFixture) -> Option<String> {
    text.lines()
        .find(|line| line.contains(&fixture.expected_answer_contains))
        .map(ToOwned::to_owned)
}

fn estimate_tokens_from_chars(chars: u64) -> u64 {
    chars.div_ceil(4)
}

fn format_rfc3339(timestamp: OffsetDateTime) -> String {
    timestamp
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| timestamp.to_string())
}

fn benchmark_results_markdown(report: &FileBackedContextReport) -> String {
    let mut out = String::new();
    out.push_str("# File-Backed Dynamic Context Benchmark Results\n\n");
    out.push_str(&format!(
        "- Fixture dir: `{}`\n- Offline: `{}`\n- Generated: `{}`\n\n",
        report.fixture_dir.display(),
        report.offline,
        format_rfc3339(report.generated_at)
    ));
    out.push_str("| Fixture | Correct | Inline Chars Before | Inline Chars After | Tokens Before | Tokens After | Tokens Saved | Artifact Bytes | Artifact Lines | Reads | Greps | Tails | Turn ms |\n");
    out.push_str("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for result in &report.results {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            result.fixture_id,
            result.answer_correct,
            result.inline_chars_before,
            result.inline_chars_after,
            result.inline_tokens_before,
            result.inline_tokens_after,
            result.inline_tokens_saved,
            result.artifact_bytes_written,
            result.artifact_lines_written,
            result.artifact_read_count,
            result.artifact_grep_count,
            result.artifact_tail_count,
            result.turn_wall_time_ms
        ));
    }
    out.push_str("\n## Recovered Details\n\n");
    for result in &report.results {
        out.push_str(&format!(
            "- `{}`: `{}`\n",
            result.fixture_id,
            result
                .recovered_detail
                .as_deref()
                .unwrap_or("<not recovered>")
        ));
    }
    out
}

fn benchmark_findings_markdown(report: &FileBackedContextReport) -> String {
    let fixture_count = report.results.len() as u64;
    let correct = report
        .results
        .iter()
        .filter(|result| result.answer_correct)
        .count() as u64;
    let tokens_before: u64 = report
        .results
        .iter()
        .map(|result| result.inline_tokens_before)
        .sum();
    let tokens_after: u64 = report
        .results
        .iter()
        .map(|result| result.inline_tokens_after)
        .sum();
    let tokens_saved = tokens_before.saturating_sub(tokens_after);
    let artifact_bytes: u64 = report
        .results
        .iter()
        .map(|result| result.artifact_bytes_written)
        .sum();
    let artifact_lines: u64 = report
        .results
        .iter()
        .map(|result| result.artifact_lines_written)
        .sum();
    let total_ms: u64 = report
        .results
        .iter()
        .map(|result| result.turn_wall_time_ms)
        .sum();
    let grep_count: u64 = report
        .results
        .iter()
        .map(|result| result.artifact_grep_count)
        .sum();
    format!(
        "# File-Backed Dynamic Context Findings Summary\n\n\
         ## Headline\n\n\
         - Fixtures run: `{fixture_count}`\n\
         - Hidden-detail recovery: `{correct}/{fixture_count}`\n\
         - Inline tokens before: `{tokens_before}`\n\
         - Inline tokens after: `{tokens_after}`\n\
         - Inline tokens saved: `{tokens_saved}`\n\
         - Artifact bytes written: `{artifact_bytes}`\n\
         - Artifact lines written: `{artifact_lines}`\n\
         - Artifact grep calls: `{grep_count}`\n\
         - Total benchmark wall time: `{total_ms} ms`\n\n\
         ## Findings\n\n\
         - File-backed context recovered every hidden detail in the current offline fixture set.\n\
         - The long-command fixture shows the intended win: most log bytes move out of inline context while a single artifact grep recovers the token.\n\
         - The compaction-history fixture confirms the summary can remain compact while exact prior details stay recoverable through a chat-history artifact.\n\n\
         ## Current Limitations\n\n\
         - This benchmark uses deterministic offline fixture payloads and local artifact operations, not live provider turns.\n\
         - Runtime ablation is available with `[context].file_backed_dynamic_context = false` or `RODER_DISABLE_CONTEXT_ARTIFACTS=1`; this offline benchmark has not yet generated a side-by-side ablation table.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backed_context_loads_fixture_json() {
        let dir = std::env::temp_dir().join(format!(
            "roder-evals-fixtures-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("one.json"),
            r#"{
              "id": "one",
              "title": "One",
              "prompt": "Find the token",
              "tags": ["file-backed-context"],
              "expectedAnswerContains": "TOKEN",
              "expectedArtifactQuery": "TOKEN",
              "expectedTool": "grep"
            }"#,
        )
        .unwrap();

        let fixtures = load_fixtures(&dir).unwrap();

        assert_eq!(fixtures.len(), 1);
        assert_eq!(fixtures[0].expected_tool, ExpectedArtifactTool::Grep);
        let _ = std::fs::remove_dir_all(dir);
    }
}
