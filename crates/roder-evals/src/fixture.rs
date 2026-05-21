use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalFixture {
    pub id: String,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub workspace: EvalWorkspaceSetup,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub expected: EvalExpectedEvidence,
    #[serde(default)]
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalWorkspaceSetup {
    #[serde(default)]
    pub files: Vec<EvalWorkspaceFile>,
    #[serde(default)]
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalWorkspaceFile {
    pub path: PathBuf,
    pub contents: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalExpectedEvidence {
    #[serde(default)]
    pub final_answer_contains: Vec<String>,
    #[serde(default)]
    pub files: Vec<EvalExpectedFile>,
    #[serde(default)]
    pub command_checks: Vec<EvalExpectedCommand>,
    #[serde(default)]
    pub verification_required: bool,
    #[serde(default)]
    pub task_ledger_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalExpectedFile {
    pub path: PathBuf,
    #[serde(default = "default_true")]
    pub exists: bool,
    #[serde(default)]
    pub contains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalExpectedCommand {
    pub command: String,
    #[serde(default)]
    pub expected_exit_code: i32,
    #[serde(default)]
    pub stdout_contains: Vec<String>,
    #[serde(default)]
    pub stderr_contains: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_fixture_round_trips_workspace_and_expected_checks() {
        let fixture = EvalFixture {
            id: "edit-config".to_string(),
            title: "Edit config".to_string(),
            prompt: "Set retries to 3 and verify tests.".to_string(),
            tags: vec!["tool-calls".to_string()],
            workspace: EvalWorkspaceSetup {
                files: vec![EvalWorkspaceFile {
                    path: PathBuf::from("config.toml"),
                    contents: "retries = 1\n".to_string(),
                }],
                commands: vec!["cargo test".to_string()],
            },
            timeout_ms: Some(30_000),
            expected: EvalExpectedEvidence {
                final_answer_contains: vec!["verified".to_string()],
                files: vec![EvalExpectedFile {
                    path: PathBuf::from("config.toml"),
                    exists: true,
                    contains: vec!["retries = 3".to_string()],
                }],
                command_checks: vec![EvalExpectedCommand {
                    command: "cargo test".to_string(),
                    expected_exit_code: 0,
                    stdout_contains: vec!["test result: ok".to_string()],
                    stderr_contains: Vec::new(),
                }],
                verification_required: true,
                task_ledger_required: true,
            },
            constraints: vec!["do not ask the user".to_string()],
        };

        let json = serde_json::to_string(&fixture).unwrap();
        let round_trip: EvalFixture = serde_json::from_str(&json).unwrap();

        assert_eq!(round_trip, fixture);
        assert_eq!(
            round_trip.workspace.files[0].path,
            PathBuf::from("config.toml")
        );
        assert!(round_trip.expected.verification_required);
    }
}
