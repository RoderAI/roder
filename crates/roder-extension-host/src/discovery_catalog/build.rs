use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use roder_api::discovery::{
    DiscoveryAuthState, DiscoveryCacheStatus, DiscoveryCatalogItem, DiscoveryCatalogSource,
    DiscoveryItemStatus, DiscoveryLifecycleState, DiscoveryPromotionRecord,
    DiscoveryPromotionState, DiscoveryRedaction, DiscoverySchemaFormat, DiscoverySchemaReference,
    DiscoverySourceKind,
};
use roder_api::extension::ExtensionRegistry;
use roder_api::skills::{SkillActivationState, SkillDescriptor, SkillExposure, SkillSource};
use roder_api::subagents::SubagentDefinition;
use roder_api::tools::ToolRegistry;
use roder_api::workflow::{WorkflowImportItem, WorkflowSourceType};
use serde::Serialize;

use super::{apply_promoted_state, group};

pub(crate) fn tool_groups(
    registry: &ExtensionRegistry,
    root: &Path,
    promoted: &[DiscoveryPromotionRecord],
) -> anyhow::Result<Vec<roder_api::discovery::DiscoveryCatalogGroup>> {
    let mut groups = Vec::new();
    for contributor in &registry.tools {
        let provider_id = contributor.id();
        let mut tools = ToolRegistry::default();
        contributor.contribute(&mut tools)?;
        let mut items = Vec::new();
        for spec in tools.specs() {
            let schema_path = format!(
                "tools/{}/{}.schema.json",
                safe_segment(&provider_id),
                safe_segment(&spec.name)
            );
            write_json(root.join(&schema_path), &spec.parameters)?;
            let mut item = spec.discovery_item(&provider_id, schema_path);
            apply_promoted_state(&mut item, promoted);
            if is_artifact_tool(&item.name) {
                item.source.kind = DiscoverySourceKind::ArtifactTools;
                item.tags.push("artifact".to_string());
                item.hints
                    .push("Inspect file-backed context artifacts by id.".to_string());
            }
            items.push(item);
        }
        items.sort_by(|left, right| left.id.cmp(&right.id));
        groups.push(group(
            "tools",
            &provider_id,
            DiscoverySourceKind::InternalTools,
            "Internal tool provider",
            items,
        ));
    }
    Ok(groups)
}

pub(crate) fn workflow_groups(
    workflow_items: &[WorkflowImportItem],
    root: &Path,
    promoted: &[DiscoveryPromotionRecord],
) -> anyhow::Result<Vec<roder_api::discovery::DiscoveryCatalogGroup>> {
    let mut grouped = BTreeMap::<DiscoverySourceKind, Vec<DiscoveryCatalogItem>>::new();
    for item in workflow_items {
        let kind = workflow_source_kind(&item.source.source_type);
        let schema_path = format!(
            "workflow-imports/{}/{}.json",
            source_kind_segment(&kind),
            safe_segment(&item.id)
        );
        write_json(root.join(&schema_path), &item.preview)?;
        let mut catalog_item = workflow_item(item, kind.clone(), schema_path);
        apply_promoted_state(&mut catalog_item, promoted);
        grouped.entry(kind).or_default().push(catalog_item);
    }

    let mut groups = Vec::new();
    for (kind, mut items) in grouped {
        items.sort_by(|left, right| left.id.cmp(&right.id));
        groups.push(group(
            "workflow-imports",
            source_kind_segment(&kind),
            kind,
            "Workflow import source",
            items,
        ));
    }
    Ok(groups)
}

pub(crate) fn subagent_groups(
    registry: &ExtensionRegistry,
    root: &Path,
    promoted: &[DiscoveryPromotionRecord],
) -> anyhow::Result<Vec<roder_api::discovery::DiscoveryCatalogGroup>> {
    let mut items = Vec::new();
    for dispatcher in &registry.subagent_dispatchers {
        let dispatcher_id = dispatcher.id();
        for definition in dispatcher.definitions() {
            let schema_path = format!(
                "subagents/{}/{}.json",
                safe_segment(&dispatcher_id),
                safe_segment(&definition.agent_type)
            );
            write_json(root.join(&schema_path), &definition)?;
            let mut item = subagent_item(&dispatcher_id, &definition, schema_path);
            apply_promoted_state(&mut item, promoted);
            items.push(item);
        }
    }
    if items.is_empty() {
        return Ok(Vec::new());
    }
    items.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(vec![group(
        "subagents",
        "dispatchers",
        DiscoverySourceKind::Subagents,
        "Subagent dispatcher definitions",
        items,
    )])
}

pub(crate) fn skill_groups(
    skills: &[SkillDescriptor],
    root: &Path,
    promoted: &[DiscoveryPromotionRecord],
) -> anyhow::Result<Vec<roder_api::discovery::DiscoveryCatalogGroup>> {
    if skills.is_empty() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    for skill in skills {
        let schema_path = format!(
            "skills/{}/{}.md",
            safe_segment(&source_label(&skill.source)),
            safe_segment(&skill.canonical_path)
        );
        write_skill_markdown(root.join(&schema_path), skill)?;
        let mut item = skill_item(skill, schema_path);
        apply_promoted_state(&mut item, promoted);
        items.push(item);
    }
    items.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(vec![group(
        "skills",
        "registry",
        DiscoverySourceKind::Skills,
        "Skill registry",
        items,
    )])
}

fn workflow_item(
    item: &WorkflowImportItem,
    kind: DiscoverySourceKind,
    schema_path: String,
) -> DiscoveryCatalogItem {
    DiscoveryCatalogItem {
        id: format!("workflow:{}", item.id),
        group_id: format!("workflow-imports:{}", source_kind_segment(&kind)),
        source: DiscoveryCatalogSource {
            kind,
            id: item
                .source
                .name
                .clone()
                .unwrap_or_else(|| item.source.path.clone()),
            display_name: item.title.clone(),
            origin: Some(item.source.path.clone()),
            auth_state: if item.approval_required {
                DiscoveryAuthState::Required
            } else {
                DiscoveryAuthState::NotRequired
            },
            redaction: redaction_for_preview(&item.preview),
        },
        name: item
            .source
            .name
            .clone()
            .unwrap_or_else(|| item.title.clone()),
        title: item.title.clone(),
        description: Some(item.summary.clone()),
        status: if item.command_capable {
            DiscoveryItemStatus::Disabled
        } else {
            DiscoveryItemStatus::Available
        },
        lifecycle: DiscoveryLifecycleState::Discovered,
        promotion: DiscoveryPromotionState::NotPromoted,
        cache_status: DiscoveryCacheStatus::Cold,
        schema: Some(DiscoverySchemaReference {
            format: DiscoverySchemaFormat::Json,
            uri: schema_path,
            content_hash: Some(item.source.hash.clone()),
            byte_count: None,
            redaction: redaction_for_preview(&item.preview),
        }),
        tags: workflow_tags(item),
        hints: workflow_hints(item),
        redaction: redaction_for_preview(&item.preview),
        last_refreshed_at: Some(item.source.detected_at),
    }
}

fn subagent_item(
    dispatcher_id: &str,
    definition: &SubagentDefinition,
    schema_path: String,
) -> DiscoveryCatalogItem {
    DiscoveryCatalogItem {
        id: format!("subagent:{dispatcher_id}/{}", definition.agent_type),
        group_id: "subagents:dispatchers".to_string(),
        source: DiscoveryCatalogSource {
            kind: DiscoverySourceKind::Subagents,
            id: dispatcher_id.to_string(),
            display_name: dispatcher_id.to_string(),
            origin: None,
            auth_state: DiscoveryAuthState::NotRequired,
            redaction: DiscoveryRedaction::none(),
        },
        name: definition.agent_type.clone(),
        title: definition.agent_type.clone(),
        description: Some(definition.description.clone()),
        status: DiscoveryItemStatus::Available,
        lifecycle: DiscoveryLifecycleState::Discovered,
        promotion: DiscoveryPromotionState::NotPromoted,
        cache_status: DiscoveryCacheStatus::Cold,
        schema: Some(DiscoverySchemaReference {
            format: DiscoverySchemaFormat::Json,
            uri: schema_path,
            content_hash: None,
            byte_count: None,
            redaction: DiscoveryRedaction::none(),
        }),
        tags: vec!["subagent".to_string()],
        hints: vec!["Promote before delegating specialized work.".to_string()],
        redaction: DiscoveryRedaction::none(),
        last_refreshed_at: None,
    }
}

fn skill_item(skill: &SkillDescriptor, schema_path: String) -> DiscoveryCatalogItem {
    DiscoveryCatalogItem {
        id: format!("skill:{}", skill.canonical_path),
        group_id: "skills:registry".to_string(),
        source: DiscoveryCatalogSource {
            kind: DiscoverySourceKind::Skills,
            id: source_label(&skill.source),
            display_name: source_title(&skill.source),
            origin: Some(skill.canonical_path.clone()),
            auth_state: DiscoveryAuthState::NotRequired,
            redaction: DiscoveryRedaction::none(),
        },
        name: skill.name.clone(),
        title: skill.name.clone(),
        description: Some(skill.description.clone()),
        status: if skill.activation == SkillActivationState::Disabled {
            DiscoveryItemStatus::Disabled
        } else {
            DiscoveryItemStatus::Available
        },
        lifecycle: DiscoveryLifecycleState::Discovered,
        promotion: DiscoveryPromotionState::NotPromoted,
        cache_status: DiscoveryCacheStatus::Cold,
        schema: Some(DiscoverySchemaReference {
            format: DiscoverySchemaFormat::Markdown,
            uri: schema_path,
            content_hash: None,
            byte_count: None,
            redaction: DiscoveryRedaction::none(),
        }),
        tags: skill_tags(skill),
        hints: skill_hints(skill),
        redaction: DiscoveryRedaction::none(),
        last_refreshed_at: None,
    }
}

fn write_skill_markdown(path: impl AsRef<Path>, skill: &SkillDescriptor) -> anyhow::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let diagnostics = if skill.diagnostics.is_empty() {
        "none".to_string()
    } else {
        skill.diagnostics.join("\n- ")
    };
    fs::write(
        path,
        format!(
            "# Skill: {}\n\nDescription: {}\n\nSource: {}\nCanonical path: {}\nActivation: {:?}\nExposure: {:?}\nDiagnostics: {}\n",
            skill.name,
            skill.description,
            source_title(&skill.source),
            skill.canonical_path,
            skill.activation,
            skill.exposure,
            diagnostics
        ),
    )?;
    Ok(())
}

fn skill_tags(skill: &SkillDescriptor) -> Vec<String> {
    let mut tags = vec![
        "skill".to_string(),
        source_label(&skill.source),
        match skill.activation {
            SkillActivationState::Enabled => "enabled".to_string(),
            SkillActivationState::Disabled => "disabled".to_string(),
            SkillActivationState::Experimental => "experimental".to_string(),
        },
        match skill.exposure {
            SkillExposure::Global => "global".to_string(),
            SkillExposure::DirectOnly => "direct-only".to_string(),
        },
    ];
    if skill.source == SkillSource::BuiltIn {
        tags.push("built-in".to_string());
    }
    tags
}

fn skill_hints(skill: &SkillDescriptor) -> Vec<String> {
    let mut hints = Vec::new();
    match skill.exposure {
        SkillExposure::Global => {
            hints.push("Visible in the global skill index when enabled.".to_string());
        }
        SkillExposure::DirectOnly => {
            hints.push("Promote or invoke directly with $skill-name before use.".to_string());
        }
    }
    if skill.activation == SkillActivationState::Disabled {
        hints.push("Disabled by skill configuration.".to_string());
    }
    hints
}

fn source_label(source: &SkillSource) -> String {
    match source {
        SkillSource::Workspace => "workspace".to_string(),
        SkillSource::User => "user".to_string(),
        SkillSource::Plugin { plugin_id } => format!("plugin-{plugin_id}"),
        SkillSource::Imported { import_id } => format!("imported-{import_id}"),
        SkillSource::BuiltIn => "built-in".to_string(),
    }
}

fn source_title(source: &SkillSource) -> String {
    match source {
        SkillSource::Workspace => "Workspace skills".to_string(),
        SkillSource::User => "User skills".to_string(),
        SkillSource::Plugin { plugin_id } => format!("Plugin skills: {plugin_id}"),
        SkillSource::Imported { import_id } => format!("Imported skill: {import_id}"),
        SkillSource::BuiltIn => "Built-in skills".to_string(),
    }
}

fn workflow_source_kind(source_type: &WorkflowSourceType) -> DiscoverySourceKind {
    match source_type {
        WorkflowSourceType::Guidance => DiscoverySourceKind::WorkflowImports,
        WorkflowSourceType::Skill => DiscoverySourceKind::Skills,
        WorkflowSourceType::SlashCommand => DiscoverySourceKind::Commands,
        WorkflowSourceType::McpServer => DiscoverySourceKind::McpTools,
        WorkflowSourceType::Hook | WorkflowSourceType::Plugin | WorkflowSourceType::Unknown => {
            DiscoverySourceKind::Plugins
        }
    }
}

fn workflow_tags(item: &WorkflowImportItem) -> Vec<String> {
    let mut tags = vec![format!("{:?}", item.source.source_type).to_ascii_lowercase()];
    if item.command_capable {
        tags.push("command-capable".to_string());
    }
    if item.approval_required {
        tags.push("approval-required".to_string());
    }
    tags
}

fn workflow_hints(item: &WorkflowImportItem) -> Vec<String> {
    if item.approval_required {
        vec!["Requires approval before activation.".to_string()]
    } else {
        vec!["Passive import can be promoted as context.".to_string()]
    }
}

fn redaction_for_preview(value: &serde_json::Value) -> DiscoveryRedaction {
    let mut fields = Vec::new();
    collect_redacted_fields(value, "$", &mut fields);
    DiscoveryRedaction {
        redacted: !fields.is_empty(),
        fields,
        secret_refs: Vec::new(),
    }
}

fn collect_redacted_fields(value: &serde_json::Value, path: &str, fields: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let child = format!("{path}.{key}");
                if value == "[redacted]" {
                    fields.push(child);
                } else {
                    collect_redacted_fields(value, &child, fields);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_redacted_fields(value, &format!("{path}[{index}]"), fields);
            }
        }
        _ => {}
    }
}

pub(crate) fn write_json(path: impl AsRef<Path>, value: &impl Serialize) -> anyhow::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

fn is_artifact_tool(name: &str) -> bool {
    matches!(name, "read_artifact" | "grep_artifact" | "tail_artifact")
}

pub(crate) fn source_kind_segment(kind: &DiscoverySourceKind) -> &'static str {
    match kind {
        DiscoverySourceKind::InternalTools => "tools",
        DiscoverySourceKind::McpTools => "mcp",
        DiscoverySourceKind::Skills => "skills",
        DiscoverySourceKind::Commands => "commands",
        DiscoverySourceKind::Plugins => "plugins",
        DiscoverySourceKind::Subagents => "subagents",
        DiscoverySourceKind::ArtifactTools => "artifact-tools",
        DiscoverySourceKind::WorkflowImports => "workflow-imports",
    }
}

pub(crate) fn safe_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}
