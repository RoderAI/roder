use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub type WorkflowImportId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowSourceType {
    Guidance,
    Skill,
    McpServer,
    SlashCommand,
    Hook,
    Plugin,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowImportState {
    Detected,
    Previewed,
    Enabled,
    Ignored,
    Disabled,
    Removed,
    Stale,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowImportRisk {
    Passive,
    ReadsWorkspace,
    StartsProcess,
    RunsHook,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSource {
    pub source_type: WorkflowSourceType,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub hash: String,
    #[serde(with = "time::serde::rfc3339")]
    pub detected_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowConflict {
    pub field: String,
    pub existing: String,
    pub incoming: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowImportItem {
    pub id: WorkflowImportId,
    pub title: String,
    pub summary: String,
    pub source: WorkflowSource,
    pub state: WorkflowImportState,
    pub risk: WorkflowImportRisk,
    #[serde(default)]
    pub command_capable: bool,
    #[serde(default)]
    pub approval_required: bool,
    #[serde(default)]
    pub preview: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<WorkflowConflict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::rfc3339::option")]
    pub enabled_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowImportDecisionKind {
    Preview,
    Enable,
    Ignore,
    Disable,
    Refresh,
    Remove,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowImportDecision {
    pub item_id: WorkflowImportId,
    pub decision: WorkflowImportDecisionKind,
    pub source_hash: String,
    #[serde(default)]
    pub approved_side_effects: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub decided_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowImportScan {
    pub workspace: String,
    pub items: Vec<WorkflowImportItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<WorkflowImportError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowImportError {
    pub path: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_import_records_are_camel_case_and_source_attributed() {
        let item = WorkflowImportItem {
            id: "skill-demo".to_string(),
            title: "Demo Skill".to_string(),
            summary: "Imported skill".to_string(),
            source: WorkflowSource {
                source_type: WorkflowSourceType::Skill,
                path: ".agents/skills/demo/SKILL.md".to_string(),
                name: Some("demo".to_string()),
                hash: "abc123".to_string(),
                detected_at: OffsetDateTime::UNIX_EPOCH,
            },
            state: WorkflowImportState::Detected,
            risk: WorkflowImportRisk::Passive,
            command_capable: false,
            approval_required: false,
            preview: serde_json::json!({ "description": "safe" }),
            conflicts: Vec::new(),
            enabled_at: None,
        };

        let value = serde_json::to_value(&item).unwrap();

        assert_eq!(value["source"]["sourceType"], "skill");
        assert_eq!(value["source"]["detectedAt"], "1970-01-01T00:00:00Z");
        assert_eq!(value["commandCapable"], false);
        assert_eq!(value["source"]["path"], ".agents/skills/demo/SKILL.md");
    }

    #[test]
    fn command_capable_imports_can_require_approval() {
        let decision = WorkflowImportDecision {
            item_id: "mcp-server-local".to_string(),
            decision: WorkflowImportDecisionKind::Enable,
            source_hash: "hash".to_string(),
            approved_side_effects: true,
            decided_at: OffsetDateTime::UNIX_EPOCH,
        };

        let value = serde_json::to_value(decision).unwrap();

        assert_eq!(value["approvedSideEffects"], true);
        assert_eq!(value["decision"], "enable");
    }
}
