use std::fs;
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::errors::path_display;
use crate::redaction::redact_sensitive_line;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightPlanSummary {
    pub path: String,
    pub exists: bool,
    pub critical_points: Vec<WebwrightCriticalPoint>,
    pub checked_count: usize,
    pub total_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightCriticalPoint {
    pub text: String,
    pub checked: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightScriptSummary {
    pub path: String,
    pub exists: bool,
    pub byte_count: u64,
    pub line_count: usize,
    pub import_safe: bool,
    pub uses_full_page_screenshot: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightLogSummary {
    pub path: String,
    pub exists: bool,
    pub line_count: usize,
    pub final_datum: Option<String>,
    pub tail: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightSelfReflectSummary {
    pub path: String,
    #[serde(default)]
    pub predicted_label: Option<String>,
    #[serde(default)]
    pub passed: Option<bool>,
    pub raw: serde_json::Value,
}

pub fn read_plan_summary(path: &Path) -> anyhow::Result<WebwrightPlanSummary> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WebwrightPlanSummary {
                path: path_display(path),
                exists: false,
                critical_points: Vec::new(),
                checked_count: 0,
                total_count: 0,
            });
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    let critical_points = text
        .lines()
        .filter_map(parse_critical_point)
        .collect::<Vec<_>>();
    let checked_count = critical_points.iter().filter(|point| point.checked).count();
    Ok(WebwrightPlanSummary {
        path: path_display(path),
        exists: true,
        total_count: critical_points.len(),
        critical_points,
        checked_count,
    })
}

pub fn read_script_summary(path: &Path) -> anyhow::Result<WebwrightScriptSummary> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WebwrightScriptSummary {
                path: path_display(path),
                exists: false,
                byte_count: 0,
                line_count: 0,
                import_safe: false,
                uses_full_page_screenshot: false,
            });
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    Ok(WebwrightScriptSummary {
        path: path_display(path),
        exists: true,
        byte_count: text.len() as u64,
        line_count: text.lines().count(),
        import_safe: text.contains("__main__"),
        uses_full_page_screenshot: uses_full_page_screenshot(&text),
    })
}

pub fn read_log_summary(path: &Path, max_tail_lines: usize) -> anyhow::Result<WebwrightLogSummary> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WebwrightLogSummary {
                path: path_display(path),
                exists: false,
                line_count: 0,
                final_datum: None,
                tail: Vec::new(),
            });
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    let lines = text.lines().map(redact_sensitive_line).collect::<Vec<_>>();
    let tail = if lines.len() > max_tail_lines {
        lines[lines.len() - max_tail_lines..].to_vec()
    } else {
        lines.clone()
    };
    let final_datum = lines
        .iter()
        .find(|line| line.to_ascii_lowercase().contains("final datum:"))
        .cloned();
    Ok(WebwrightLogSummary {
        path: path_display(path),
        exists: true,
        line_count: lines.len(),
        final_datum,
        tail,
    })
}

pub fn read_optional_self_reflect(
    path: &Path,
) -> anyhow::Result<Option<WebwrightSelfReflectSummary>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let raw: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(WebwrightSelfReflectSummary {
        path: path_display(path),
        predicted_label: raw
            .get("predicted_label")
            .or_else(|| raw.get("predictedLabel"))
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        passed: raw.get("passed").and_then(|value| value.as_bool()),
        raw,
    }))
}

pub fn uses_full_page_screenshot(text: &str) -> bool {
    text.contains("full_page=True")
        || text.contains("full_page = True")
        || text.contains("fullPage: true")
}

fn parse_critical_point(line: &str) -> Option<WebwrightCriticalPoint> {
    let trimmed = line.trim();
    let checked = if let Some(rest) = trimmed.strip_prefix("- [x]") {
        Some((true, rest))
    } else if let Some(rest) = trimmed.strip_prefix("- [X]") {
        Some((true, rest))
    } else if let Some(rest) = trimmed.strip_prefix("- [ ]") {
        Some((false, rest))
    } else {
        None
    }?;
    Some(WebwrightCriticalPoint {
        checked: checked.0,
        text: checked.1.trim().to_string(),
    })
}
