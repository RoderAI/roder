use roder_api::workflow::{
    WorkflowConflict, WorkflowImportItem, WorkflowImportRisk, WorkflowSourceType,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedWorkflowImport {
    pub item_id: String,
    pub target: WorkflowImportTarget,
    pub disabled_by_default: bool,
    pub approval_required: bool,
    pub conflicts: Vec<WorkflowConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowImportTarget {
    ContextProvider { name: String, source_path: String },
    Skill { name: String, source_path: String },
    Command { name: String, source_path: String },
    McpServer { name: String, source_path: String },
    Hook { name: String, source_path: String },
    Plugin { name: String, source_path: String },
}

pub fn normalize_workflow_imports(items: &[WorkflowImportItem]) -> Vec<NormalizedWorkflowImport> {
    let mut seen_names = std::collections::HashMap::<(WorkflowSourceType, String), String>::new();
    items
        .iter()
        .map(|item| {
            let name = normalized_name(item);
            let key = (item.source.source_type.clone(), name.clone());
            let mut conflicts = item.conflicts.clone();
            if let Some(existing) = seen_names.insert(key, item.id.clone()) {
                conflicts.push(WorkflowConflict {
                    field: "name".to_string(),
                    existing,
                    incoming: item.id.clone(),
                    detail: format!("duplicate workflow import name {name}"),
                });
            }
            let source_path = item.source.path.clone();
            NormalizedWorkflowImport {
                item_id: item.id.clone(),
                target: target_for(item, name, source_path),
                disabled_by_default: item.command_capable
                    || matches!(
                        item.risk,
                        WorkflowImportRisk::StartsProcess
                            | WorkflowImportRisk::RunsHook
                            | WorkflowImportRisk::Unknown
                    ),
                approval_required: item.approval_required || item.command_capable,
                conflicts,
            }
        })
        .collect()
}

fn target_for(
    item: &WorkflowImportItem,
    name: String,
    source_path: String,
) -> WorkflowImportTarget {
    match item.source.source_type {
        WorkflowSourceType::Guidance => WorkflowImportTarget::ContextProvider { name, source_path },
        WorkflowSourceType::Skill => WorkflowImportTarget::Skill { name, source_path },
        WorkflowSourceType::SlashCommand => WorkflowImportTarget::Command { name, source_path },
        WorkflowSourceType::McpServer => WorkflowImportTarget::McpServer { name, source_path },
        WorkflowSourceType::Hook => WorkflowImportTarget::Hook { name, source_path },
        WorkflowSourceType::Plugin | WorkflowSourceType::Unknown => {
            WorkflowImportTarget::Plugin { name, source_path }
        }
    }
}

fn normalized_name(item: &WorkflowImportItem) -> String {
    item.source
        .name
        .clone()
        .unwrap_or_else(|| item.title.clone())
        .trim_start_matches('/')
        .to_ascii_lowercase()
        .replace(' ', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::workflow::{WorkflowImportState, WorkflowSource, WorkflowSourceType};
    use time::OffsetDateTime;

    #[test]
    fn command_capable_imports_are_disabled_and_approval_gated() {
        let items = vec![item("mcp-a", WorkflowSourceType::McpServer, true)];

        let normalized = normalize_workflow_imports(&items);

        assert_eq!(normalized.len(), 1);
        assert!(normalized[0].disabled_by_default);
        assert!(normalized[0].approval_required);
        assert!(matches!(
            normalized[0].target,
            WorkflowImportTarget::McpServer { .. }
        ));
    }

    #[test]
    fn duplicate_names_are_reported_as_conflicts() {
        let items = vec![
            item("skill-a", WorkflowSourceType::Skill, false),
            item("skill-a", WorkflowSourceType::Skill, false),
        ];

        let normalized = normalize_workflow_imports(&items);

        assert_eq!(normalized[1].conflicts.len(), 1);
    }

    fn item(
        name: &str,
        source_type: WorkflowSourceType,
        command_capable: bool,
    ) -> WorkflowImportItem {
        WorkflowImportItem {
            id: format!("id-{name}-{}", command_capable as u8),
            title: name.to_string(),
            summary: "summary".to_string(),
            source: WorkflowSource {
                source_type,
                path: format!("{name}.md"),
                name: Some(name.to_string()),
                hash: "hash".to_string(),
                detected_at: OffsetDateTime::UNIX_EPOCH,
            },
            state: WorkflowImportState::Detected,
            risk: if command_capable {
                WorkflowImportRisk::StartsProcess
            } else {
                WorkflowImportRisk::Passive
            },
            command_capable,
            approval_required: command_capable,
            preview: serde_json::Value::Null,
            conflicts: Vec::new(),
            enabled_at: None,
        }
    }
}
