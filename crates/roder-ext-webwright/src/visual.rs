use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::errors::path_display;
use crate::redaction::redact_sensitive_text;
use crate::verify::verify_workspace;
use crate::workspace::{WebwrightRunSummary, WebwrightWorkspace};

pub const VISUAL_JUDGE_DIR: &str = "visual_judge";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightVisualJudgeRecord {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    pub record_path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebwrightPreparedVisualJudge {
    pub run_id: u32,
    pub screenshot_path: PathBuf,
    pub prompt: String,
    pub record_path: PathBuf,
}

impl WebwrightVisualJudgeRecord {
    pub fn skipped(
        workspace: &WebwrightWorkspace,
        run_id: Option<u32>,
        provider: impl Into<String>,
        model: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(
            workspace,
            run_id,
            "skipped",
            None,
            Some(reason.into()),
            provider,
            model,
            None,
            None,
            None,
        )
    }

    pub fn completed(
        prepared: &WebwrightPreparedVisualJudge,
        provider: impl Into<String>,
        model: impl Into<String>,
        response: impl Into<String>,
    ) -> Self {
        let response = redact_sensitive_text(&response.into());
        let passed = infer_visual_judge_passed(&response);
        Self::new(
            &WebwrightWorkspace::new(
                prepared
                    .record_path
                    .parent()
                    .and_then(Path::parent)
                    .unwrap_or_else(|| Path::new(".")),
            ),
            Some(prepared.run_id),
            "completed",
            passed,
            None,
            provider,
            model,
            Some(path_display(&prepared.screenshot_path)),
            Some(prepared.prompt.clone()),
            Some(response),
        )
    }

    pub fn failed(
        prepared: Option<&WebwrightPreparedVisualJudge>,
        workspace: &WebwrightWorkspace,
        run_id: Option<u32>,
        provider: impl Into<String>,
        model: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        let (run_id, screenshot_path, prompt) = prepared
            .map(|prepared| {
                (
                    Some(prepared.run_id),
                    Some(path_display(&prepared.screenshot_path)),
                    Some(prepared.prompt.clone()),
                )
            })
            .unwrap_or((run_id, None, None));
        Self::new(
            workspace,
            run_id,
            "failed",
            Some(false),
            Some(reason.into()),
            provider,
            model,
            screenshot_path,
            prompt,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        workspace: &WebwrightWorkspace,
        run_id: Option<u32>,
        status: impl Into<String>,
        passed: Option<bool>,
        reason: Option<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        screenshot_path: Option<String>,
        prompt: Option<String>,
        response: Option<String>,
    ) -> Self {
        let record_path = visual_judge_record_path(workspace.root(), run_id);
        Self {
            status: status.into(),
            passed,
            reason: reason.map(|text| redact_sensitive_text(&text)),
            provider: provider.into(),
            model: model.into(),
            run_id,
            screenshot_path,
            prompt: prompt.map(|text| redact_sensitive_text(&text)),
            response: response.map(|text| redact_sensitive_text(&text)),
            record_path: path_display(&record_path),
            created_at: time::OffsetDateTime::now_utc().unix_timestamp().to_string(),
        }
    }
}

pub fn prepare_visual_judge(
    workspace: &WebwrightWorkspace,
    run_id: Option<u32>,
) -> anyhow::Result<WebwrightPreparedVisualJudge> {
    let summary = workspace.summary()?;
    let selected_run_id = run_id
        .or(summary.latest_run)
        .context("missing latest Webwright run for visual judge")?;
    let run = summary
        .runs
        .iter()
        .find(|run| run.run_id == selected_run_id)
        .with_context(|| format!("missing Webwright run {selected_run_id} for visual judge"))?;
    let screenshot_path = run
        .screenshots
        .first()
        .map(PathBuf::from)
        .with_context(|| format!("run {selected_run_id} has no final_execution screenshot"))?;
    Ok(WebwrightPreparedVisualJudge {
        run_id: selected_run_id,
        screenshot_path,
        prompt: build_visual_judge_prompt(workspace, run),
        record_path: visual_judge_record_path(workspace.root(), Some(selected_run_id)),
    })
}

pub fn store_visual_judge_record(
    record: WebwrightVisualJudgeRecord,
) -> anyhow::Result<WebwrightVisualJudgeRecord> {
    let path = Path::new(&record.record_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_string_pretty(&record)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(record)
}

pub fn visual_judge_record_path(root: &Path, run_id: Option<u32>) -> PathBuf {
    let file = run_id
        .map(|id| format!("run_{id:03}.json"))
        .unwrap_or_else(|| "latest.json".to_string());
    root.join(VISUAL_JUDGE_DIR).join(file)
}

fn build_visual_judge_prompt(workspace: &WebwrightWorkspace, run: &WebwrightRunSummary) -> String {
    let summary = workspace.summary().ok();
    let verification = verify_workspace(workspace.root());
    let critical_points = summary
        .as_ref()
        .map(|summary| {
            summary
                .plan
                .critical_points
                .iter()
                .map(|point| {
                    format!(
                        "- [{}] {}",
                        if point.checked { "x" } else { " " },
                        point.text
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "- no critical points found".to_string());
    let checks = verification
        .checks
        .iter()
        .map(|check| {
            format!(
                "- {}: {} ({})",
                check.id,
                if check.passed { "passed" } else { "failed" },
                check.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let final_datum = run.log.final_datum.as_deref().unwrap_or("missing");
    redact_sensitive_text(&format!(
        "You are Roder's optional Webwright visual judge.\n\
         Inspect the attached final_execution screenshot for the Webwright run.\n\
         Return a concise JSON object with keys passed, observations, and concerns.\n\n\
         Critical points:\n{critical_points}\n\n\
         Deterministic checks:\n{checks}\n\n\
         Final datum: {final_datum}"
    ))
}

fn infer_visual_judge_passed(response: &str) -> Option<bool> {
    let lower = response.to_ascii_lowercase();
    if lower.contains("\"passed\": true") || lower.contains("passed: true") {
        Some(true)
    } else if lower.contains("\"passed\": false") || lower.contains("passed: false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::WebwrightManifest;

    fn tempdir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "roder-webwright-visual-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn stores_redacted_visual_judge_records_under_workspace() {
        let root = tempdir("record");
        let workspace = WebwrightWorkspace::new(&root);
        workspace
            .create(&WebwrightManifest::new(
                "visual",
                "Open the page",
                crate::workspace::WebwrightMode::Run,
                None,
                None,
                true,
            ))
            .unwrap();

        let record = WebwrightVisualJudgeRecord::skipped(
            &workspace,
            None,
            "mock",
            "mock",
            "Authorization: Bearer secret",
        );
        let stored = store_visual_judge_record(record).unwrap();

        assert!(Path::new(&stored.record_path).is_file());
        let text = fs::read_to_string(stored.record_path).unwrap();
        assert!(text.contains("[redacted sensitive Webwright output line]"));
        assert!(!text.contains("secret"));
    }

    #[test]
    fn infers_boolean_passed_from_jsonish_response() {
        assert_eq!(infer_visual_judge_passed(r#"{"passed": true}"#), Some(true));
        assert_eq!(infer_visual_judge_passed("passed: false"), Some(false));
        assert_eq!(infer_visual_judge_passed("unclear"), None);
    }
}
