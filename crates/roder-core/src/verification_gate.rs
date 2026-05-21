use std::collections::BTreeSet;

use roder_api::conversation::ToolResultRecord;
use roder_api::inference::RuntimeProfile;

pub(crate) const VERIFICATION_TOOL_NAME: &str = "verification.review";

#[derive(Debug, Clone)]
pub(crate) struct VerificationGateState {
    original_task: String,
    enabled: bool,
    required: bool,
    changed_files: BTreeSet<String>,
    pub(crate) tool_evidence: Vec<String>,
    pub(crate) tests_run: Vec<String>,
    completed: bool,
    pub(crate) open_gaps: Vec<String>,
}

impl VerificationGateState {
    pub(crate) fn new(original_task: String, profile: RuntimeProfile) -> Self {
        Self {
            original_task,
            enabled: profile == RuntimeProfile::Eval,
            required: false,
            changed_files: BTreeSet::new(),
            tool_evidence: Vec::new(),
            tests_run: Vec::new(),
            completed: false,
            open_gaps: Vec::new(),
        }
    }

    pub(crate) fn record_tool_result(&mut self, result: &ToolResultRecord) {
        let name = result.name.as_deref().unwrap_or_default();
        if name == VERIFICATION_TOOL_NAME {
            self.record_verification_result(result);
            return;
        }
        if is_code_change_tool(name) && !result.is_error {
            self.required = true;
            self.completed = false;
            if let Some(path) = result
                .display_payload
                .as_ref()
                .and_then(|payload| payload.get("path"))
                .and_then(serde_json::Value::as_str)
                .filter(|path| !path.trim().is_empty())
            {
                self.changed_files.insert(path.to_string());
            }
        }
        if name == "exec_command" && result.result.contains("test") {
            self.tests_run.push(trim_evidence(&result.result));
        }
        if !name.is_empty() && !result.is_error {
            self.tool_evidence
                .push(format!("{name}: {}", trim_evidence(&result.result)));
        }
    }

    fn record_verification_result(&mut self, result: &ToolResultRecord) {
        if result.is_error {
            self.completed = false;
            self.open_gaps = vec![trim_evidence(&result.result)];
            return;
        }
        if result.result.starts_with("Verification completed") {
            self.completed = true;
            self.open_gaps.clear();
        } else if result.result.starts_with("Verification failed:") {
            self.completed = false;
            self.open_gaps = result
                .result
                .trim_start_matches("Verification failed:")
                .split(';')
                .map(str::trim)
                .filter(|gap| !gap.is_empty())
                .map(str::to_string)
                .collect();
        } else if result.result.starts_with("Verification skipped:") && !self.required {
            self.completed = true;
        }
    }

    pub(crate) fn blocking_prompt(&self) -> Option<String> {
        if !self.enabled || !self.required || self.completed {
            return None;
        }
        let changed_files = join_or_none(self.changed_files());
        let tool_evidence = join_or_none(self.tool_evidence.clone());
        let tests_run = join_or_none(self.tests_run.clone());
        let open_gaps = join_or_none(self.open_gaps.clone());
        Some(format!(
            "Verification gate blocked final completion. Before answering, call `{VERIFICATION_TOOL_NAME}` with the original task, changed files, tool evidence, tests run, and any open gaps. If verification reports gaps, address them and call `{VERIFICATION_TOOL_NAME}` again before finalizing.\n\nOriginal task: {}\nChanged files: {changed_files}\nTool evidence: {tool_evidence}\nTests run: {tests_run}\nOpen gaps: {open_gaps}",
            self.original_task
        ))
    }

    pub(crate) fn reason(&self) -> String {
        if self.open_gaps.is_empty() {
            "code_changes_without_verification".to_string()
        } else {
            "verification_gaps_remaining".to_string()
        }
    }

    pub(crate) fn changed_files(&self) -> Vec<String> {
        self.changed_files.iter().cloned().collect()
    }
}

fn is_code_change_tool(name: &str) -> bool {
    matches!(name, "write_file" | "edit" | "multi_edit" | "apply_patch")
}

fn trim_evidence(text: &str) -> String {
    const MAX: usize = 240;
    let text = text.trim().replace('\n', " ");
    if text.chars().count() <= MAX {
        text
    } else {
        format!("{}...", text.chars().take(MAX).collect::<String>())
    }
}

fn join_or_none(values: Vec<String>) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join("; ")
    }
}
