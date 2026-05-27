use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::redaction::redact_sensitive_line;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebwrightTaskDefinition {
    pub task_id: String,
    #[serde(default)]
    pub short_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub cadence: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    pub task_prompt: String,
    #[serde(default)]
    pub num_steps: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebwrightReport {
    #[serde(default)]
    pub sources: Vec<ReportSource>,
    pub result: ReportResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportSource {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportResult {
    #[serde(default)]
    pub headline: Option<String>,
    #[serde(default)]
    pub sections: Vec<ReportSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportSection {
    #[serde(rename = "type")]
    pub section_type: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Vec<String>>,
    #[serde(default)]
    pub entries: Vec<String>,
}

pub fn read_task_definition(path: &Path) -> anyhow::Result<WebwrightTaskDefinition> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read Webwright task definition {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("parse Webwright task definition {}", path.display()))
}

pub fn read_report(path: &Path) -> anyhow::Result<WebwrightReport> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read Webwright report {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("parse Webwright report {}", path.display()))
}

pub fn render_report_text(report: &WebwrightReport) -> String {
    let mut lines = Vec::new();
    if let Some(headline) = report.result.headline.as_deref() {
        push_line(&mut lines, format!("# {headline}"));
        lines.push(String::new());
    }
    if !report.sources.is_empty() {
        push_line(&mut lines, "Sources".to_string());
        for source in &report.sources {
            let note = source
                .note
                .as_deref()
                .map(|note| format!(" - {note}"))
                .unwrap_or_default();
            push_line(
                &mut lines,
                format!("- {}: {}{}", source.name, source.url, note),
            );
        }
        lines.push(String::new());
    }
    for section in &report.result.sections {
        push_line(&mut lines, format!("## {}", section.title));
        if let Some(body) = section.body.as_deref() {
            push_line(&mut lines, body.to_string());
        }
        if !section.entries.is_empty() {
            for entry in &section.entries {
                push_line(&mut lines, format!("- {entry}"));
            }
        }
        if !section.columns.is_empty() {
            push_line(&mut lines, section.columns.join(" | "));
            push_line(
                &mut lines,
                section
                    .columns
                    .iter()
                    .map(|_| "---")
                    .collect::<Vec<_>>()
                    .join(" | "),
            );
            for row in &section.rows {
                push_line(&mut lines, row.join(" | "));
            }
        }
        lines.push(String::new());
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn push_line(lines: &mut Vec<String>, line: String) {
    lines.push(redact_sensitive_line(&line));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(path: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../evals/fixtures/webwright/basic_success")
            .join(path)
    }

    #[test]
    fn parses_task2ui_fixture_report() {
        let task = read_task_definition(&fixture("task.json")).unwrap();
        let report = read_report(&fixture("report.json")).unwrap();

        assert_eq!(task.short_id.as_deref(), Some("basic_success"));
        assert_eq!(report.result.headline.as_deref(), Some("Fixture result"));
        assert_eq!(report.result.sections[0].section_type, "summary");
    }

    #[test]
    fn renders_task2ui_fixture_report_as_text() {
        let report = read_report(&fixture("report.json")).unwrap();
        let rendered = render_report_text(&report);

        assert!(rendered.contains("# Fixture result"));
        assert!(rendered.contains("## Final datum"));
        assert!(rendered.contains("Fixture Heading"));
    }
}
