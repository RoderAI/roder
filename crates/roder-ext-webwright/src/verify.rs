use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::redaction::redact_sensitive_line;
use crate::workspace::{FINAL_LOG_FILE, PLAN_FILE, WebwrightWorkspace};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerificationResult {
    #[serde(rename = "predicted_label")]
    pub predicted_label: String,
    pub passed: bool,
    pub checks: Vec<VerificationCheck>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerificationCheck {
    pub id: String,
    pub passed: bool,
    pub message: String,
}

pub fn verify_workspace(root: impl AsRef<Path>) -> VerificationResult {
    let root = root.as_ref();
    let workspace = WebwrightWorkspace::new(root);
    let mut checks = match workspace.summary() {
        Ok(summary) => {
            let mut checks = if summary.validation_errors.is_empty() {
                vec![VerificationCheck {
                    id: "workspace_contract".to_string(),
                    passed: true,
                    message: "required Webwright artifacts are present".to_string(),
                }]
            } else {
                summary
                    .validation_errors
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(index, message)| VerificationCheck {
                        id: format!("workspace_contract_{}", index + 1),
                        passed: false,
                        message,
                    })
                    .collect::<Vec<_>>()
            };
            checks.push(critical_points_check(&root.join(PLAN_FILE)));
            checks.push(screenshot_count_check(&summary));
            checks.push(final_datum_check(&workspace));
            checks
        }
        Err(err) => vec![VerificationCheck {
            id: "workspace_summary".to_string(),
            passed: false,
            message: err.to_string(),
        }],
    };
    checks.sort_by(|a, b| a.id.cmp(&b.id));
    let passed = checks.iter().all(|check| check.passed);
    VerificationResult {
        predicted_label: if passed { "success" } else { "failure" }.to_string(),
        passed,
        checks,
    }
}

fn critical_points_check(path: &Path) -> VerificationCheck {
    let Ok(text) = fs::read_to_string(path) else {
        return VerificationCheck {
            id: "critical_points".to_string(),
            passed: false,
            message: format!("missing critical point plan: {}", path.display()),
        };
    };
    let points = text
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("- [") {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if points.is_empty() {
        return VerificationCheck {
            id: "critical_points".to_string(),
            passed: false,
            message: "plan.md has no critical point checklist items".to_string(),
        };
    }
    let incomplete = points
        .iter()
        .filter(|point| !point.starts_with("- [x]") && !point.starts_with("- [X]"))
        .cloned()
        .collect::<Vec<_>>();
    VerificationCheck {
        id: "critical_points".to_string(),
        passed: incomplete.is_empty(),
        message: if incomplete.is_empty() {
            format!("{} critical point(s) checked", points.len())
        } else {
            format!("unchecked critical point(s): {}", incomplete.join("; "))
        },
    }
}

fn screenshot_count_check(
    summary: &crate::workspace::WebwrightWorkspaceSummary,
) -> VerificationCheck {
    let screenshot_count = summary
        .runs
        .iter()
        .find(|run| Some(run.run_id) == summary.latest_run)
        .map(|run| run.screenshots.len())
        .unwrap_or_default();
    VerificationCheck {
        id: "screenshot_count".to_string(),
        passed: screenshot_count > 0,
        message: if screenshot_count > 0 {
            format!("latest run has {screenshot_count} final_execution screenshot(s)")
        } else {
            "latest run has no final_execution screenshots".to_string()
        },
    }
}

fn final_datum_check(workspace: &WebwrightWorkspace) -> VerificationCheck {
    let Ok(Some(run_id)) = workspace.latest_run_id() else {
        return VerificationCheck {
            id: "final_datum".to_string(),
            passed: false,
            message: "missing latest Webwright run for final datum check".to_string(),
        };
    };
    let log_path = workspace.run_dir(run_id).join(FINAL_LOG_FILE);
    let Ok(text) = fs::read_to_string(&log_path) else {
        return VerificationCheck {
            id: "final_datum".to_string(),
            passed: false,
            message: format!("missing final datum log: {}", log_path.display()),
        };
    };
    let datum = text
        .lines()
        .find(|line| line.to_ascii_lowercase().contains("final datum:"))
        .map(str::trim);
    VerificationCheck {
        id: "final_datum".to_string(),
        passed: datum.is_some_and(|line| line.len() > "final datum:".len()),
        message: datum
            .map(|line| format!("found {}", redact_sensitive_line(line)))
            .unwrap_or_else(|| format!("missing `final datum:` line in {}", log_path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(path: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../evals/fixtures/webwright")
            .join(path)
    }

    #[test]
    fn verifies_fixture_success_and_failure() {
        let success = verify_workspace(fixture("basic_success"));
        assert!(success.passed);
        assert_eq!(success.predicted_label, "success");

        let failure = verify_workspace(fixture("missing_log"));
        assert!(!failure.passed);
        assert_eq!(failure.predicted_label, "failure");
        assert!(
            failure
                .checks
                .iter()
                .any(|check| check.id == "final_datum" && !check.passed)
        );
    }

    #[test]
    fn rejects_unchecked_critical_points() {
        let failure = verify_workspace(fixture("unchecked_plan"));
        assert!(!failure.passed);
        assert!(failure.checks.iter().any(|check| {
            check.id == "critical_points" && check.message.contains("unchecked critical point")
        }));
    }
}
