#![allow(dead_code)]

use ratatui::{
    style::Modifier,
    text::{Line, Span},
};
use roder_api::workflow::{WorkflowImportItem, WorkflowImportRisk, WorkflowImportState};

use super::Theme;

#[derive(Debug, Clone, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) struct WorkflowImportRow {
    item: WorkflowImportItem,
}

#[allow(dead_code)]
impl WorkflowImportRow {
    pub fn new(item: WorkflowImportItem) -> Self {
        Self { item }
    }

    pub fn item_id(&self) -> &str {
        &self.item.id
    }

    pub fn render(
        &self,
        selected: bool,
        expanded: bool,
        theme: Theme,
        lines: &mut Vec<Line<'static>>,
    ) {
        let title_style = if selected {
            theme.text().add_modifier(Modifier::BOLD)
        } else {
            theme.text()
        };
        lines.push(Line::from(vec![
            Span::styled("  ⧉ ", risk_style(&self.item.risk, theme)),
            Span::styled(
                format!(
                    "workflow import: {} ({})",
                    self.item.title,
                    state_label(&self.item.state)
                ),
                title_style,
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    ", theme.subtle()),
            Span::styled(
                format!(
                    "{:?} · {} · {}",
                    self.item.source.source_type,
                    risk_label(&self.item.risk),
                    self.item.source.path
                ),
                theme.muted(),
            ),
        ]));
        if self.item.approval_required {
            lines.push(Line::from(Span::styled(
                "    approval required before enabling side effects",
                theme.error(),
            )));
        }
        if !self.item.conflicts.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("    {} conflicts", self.item.conflicts.len()),
                theme.error(),
            )));
        }
        if expanded {
            lines.push(Line::from(Span::styled(
                format!("    {}", self.item.summary),
                theme.muted(),
            )));
            for line in serde_json::to_string_pretty(&self.item.preview)
                .unwrap_or_default()
                .lines()
                .take(12)
            {
                lines.push(Line::from(vec![
                    Span::styled("    ", theme.subtle()),
                    Span::styled(line.to_string(), theme.subtle()),
                ]));
            }
        }
    }
}

fn state_label(state: &WorkflowImportState) -> &'static str {
    match state {
        WorkflowImportState::Detected => "detected",
        WorkflowImportState::Previewed => "previewed",
        WorkflowImportState::Enabled => "enabled",
        WorkflowImportState::Ignored => "ignored",
        WorkflowImportState::Disabled => "disabled",
        WorkflowImportState::Removed => "removed",
        WorkflowImportState::Stale => "stale",
        WorkflowImportState::Failed => "failed",
    }
}

fn risk_label(risk: &WorkflowImportRisk) -> &'static str {
    match risk {
        WorkflowImportRisk::Passive => "passive",
        WorkflowImportRisk::ReadsWorkspace => "reads workspace",
        WorkflowImportRisk::StartsProcess => "starts process",
        WorkflowImportRisk::RunsHook => "runs hook",
        WorkflowImportRisk::Unknown => "unknown risk",
    }
}

fn risk_style(risk: &WorkflowImportRisk, theme: Theme) -> ratatui::style::Style {
    match risk {
        WorkflowImportRisk::Passive => theme.tool(),
        WorkflowImportRisk::ReadsWorkspace => theme.accent_soft(),
        WorkflowImportRisk::StartsProcess | WorkflowImportRisk::RunsHook => theme.error(),
        WorkflowImportRisk::Unknown => theme.muted(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::workflow::{WorkflowSource, WorkflowSourceType};
    use time::OffsetDateTime;

    #[test]
    fn workflow_import_row_renders_risk_and_redacted_preview() {
        let row = WorkflowImportRow::new(WorkflowImportItem {
            id: "workflow-a".to_string(),
            title: "local MCP".to_string(),
            summary: "MCP server import remains disabled.".to_string(),
            source: WorkflowSource {
                source_type: WorkflowSourceType::McpServer,
                path: ".mcp.json".to_string(),
                name: Some("local".to_string()),
                hash: "hash".to_string(),
                detected_at: OffsetDateTime::UNIX_EPOCH,
            },
            state: WorkflowImportState::Previewed,
            risk: WorkflowImportRisk::StartsProcess,
            command_capable: true,
            approval_required: true,
            preview: serde_json::json!({ "command": "node", "env": "[redacted]" }),
            conflicts: Vec::new(),
            enabled_at: None,
        });
        let mut lines = Vec::new();

        row.render(true, true, Theme::for_terminal(), &mut lines);
        let text = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("approval required"));
        assert!(text.contains("[redacted]"));
    }
}
