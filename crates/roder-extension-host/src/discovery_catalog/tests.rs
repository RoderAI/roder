use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use roder_api::discovery::{
    DiscoveryAuthState, DiscoveryCacheStatus, DiscoveryItemStatus, DiscoveryLifecycleState,
    DiscoveryPromotionRecord, DiscoveryPromotionState, DiscoverySourceKind,
};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::skills::{SkillActivationState, SkillDescriptor, SkillExposure, SkillSource};
use roder_api::subagents::{
    SubagentDefinition, SubagentDispatcher, SubagentPermissionMode, SubagentRequest, SubagentResult,
};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_api::workflow::{
    WorkflowImportItem, WorkflowImportRisk, WorkflowImportState, WorkflowSource, WorkflowSourceType,
};
use time::OffsetDateTime;

use super::*;

struct TestToolContributor;

impl ToolContributor for TestToolContributor {
    fn id(&self) -> roder_api::extension::ToolProviderId {
        "fixture-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(TestTool {
            name: "grep_artifact",
        }))?;
        registry.register(Arc::new(TestTool {
            name: "search_repo",
        }))
    }
}

struct TestTool {
    name: &'static str,
}

#[async_trait::async_trait]
impl ToolExecutor for TestTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.to_string(),
            description: format!("{} description", self.name),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        _call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        unreachable!("discovery tests do not execute tools")
    }
}

struct TestDispatcher;

#[async_trait::async_trait]
impl SubagentDispatcher for TestDispatcher {
    fn id(&self) -> roder_api::extension::SubagentDispatcherId {
        "fixture-dispatcher".to_string()
    }

    fn definitions(&self) -> Vec<SubagentDefinition> {
        vec![SubagentDefinition {
            agent_type: "reviewer".to_string(),
            description: "Review code".to_string(),
            tools: vec!["read_file".to_string()],
            model: None,
            system_prompt: None,
            permission_mode: SubagentPermissionMode::ReadOnly,
            max_turns: Some(3),
            max_result_chars: Some(1024),
        }]
    }

    async fn dispatch(
        &self,
        _parent_thread_id: String,
        _parent_turn_id: String,
        _request: SubagentRequest,
    ) -> anyhow::Result<SubagentResult> {
        unreachable!("discovery tests do not dispatch subagents")
    }
}

#[test]
fn catalog_builder_writes_grouped_tool_schema_and_artifact_tools() {
    let root = temp_dir("catalog-tools");
    let mut builder = ExtensionRegistryBuilder::new();
    builder.tool_contributor(Arc::new(TestToolContributor));
    let registry = builder.build().expect("build registry");

    let result = build_file_backed_catalog(
        &registry,
        &[],
        &DiscoveryCatalogBuildOptions::new(root.join("catalog"), root.join("session")),
    )
    .expect("build catalog");

    assert!(root.join("catalog/index.json").exists());
    assert!(
        root.join("catalog/tools/fixture-tools/grep_artifact.schema.json")
            .exists()
    );
    let tool_group = result
        .catalog
        .groups
        .iter()
        .find(|group| group.id == "tools:fixture-tools")
        .expect("tool group");
    assert_eq!(tool_group.item_count, 2);
    let artifact = tool_group
        .items
        .iter()
        .find(|item| item.name == "grep_artifact")
        .expect("artifact tool");
    assert_eq!(artifact.source.kind, DiscoverySourceKind::ArtifactTools);
    assert!(artifact.tags.contains(&"artifact".to_string()));
}

#[test]
fn workflow_imports_are_grouped_redacted_and_statused() {
    let root = temp_dir("catalog-workflows");
    let registry = ExtensionRegistryBuilder::new().build().expect("registry");
    let items = vec![workflow_item(
        "mcp-local",
        WorkflowSourceType::McpServer,
        true,
        serde_json::json!({"env":"[redacted]"}),
    )];

    let result = build_file_backed_catalog(
        &registry,
        &items,
        &DiscoveryCatalogBuildOptions::new(root.join("catalog"), root.join("session")),
    )
    .expect("build catalog");

    let mcp_group = result
        .catalog
        .groups
        .iter()
        .find(|group| group.source.kind == DiscoverySourceKind::McpTools)
        .expect("mcp group");
    let item = &mcp_group.items[0];
    assert_eq!(item.status, DiscoveryItemStatus::Disabled);
    assert_eq!(item.source.auth_state, DiscoveryAuthState::Required);
    assert!(item.redaction.redacted);
    assert!(item.redaction.fields.contains(&"$.env".to_string()));
    assert!(
        root.join("catalog/workflow-imports/mcp/mcp-local.json")
            .exists()
    );

    let path = write_catalog_group(root.join("catalog"), mcp_group).expect("rewrite one group");
    assert_eq!(path, root.join("catalog/mcp/mcp/index.json"));
    assert!(path.exists());
}

#[test]
fn subagent_definitions_and_promotions_are_persisted() {
    let root = temp_dir("catalog-subagents");
    let mut builder = ExtensionRegistryBuilder::new();
    builder.subagent_dispatcher(Arc::new(TestDispatcher));
    let registry = builder.build().expect("build registry");
    let store = PromotionStore::new(root.join("session"));
    store
        .save(&[DiscoveryPromotionRecord {
            item_id: "subagent:fixture-dispatcher/reviewer".to_string(),
            group_id: "subagents:dispatchers".to_string(),
            thread_id: "thread-a".to_string(),
            turn_id: Some("turn-a".to_string()),
            promotion: DiscoveryPromotionState::WarmCacheHit,
            cache_status: DiscoveryCacheStatus::Hit,
            reused_count: 2,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        }])
        .expect("save promotion");

    let result = build_file_backed_catalog(
        &registry,
        &[],
        &DiscoveryCatalogBuildOptions::new(root.join("catalog"), root.join("session")),
    )
    .expect("build catalog");

    let subagent = result
        .catalog
        .groups
        .iter()
        .flat_map(|group| group.items.iter())
        .find(|item| item.id == "subagent:fixture-dispatcher/reviewer")
        .expect("subagent item");
    assert_eq!(subagent.lifecycle, DiscoveryLifecycleState::WarmCached);
    assert_eq!(subagent.cache_status, DiscoveryCacheStatus::Hit);
    assert!(root.join("session/discovery/promotions.json").exists());
}

#[test]
fn skills_are_grouped_with_enabled_and_exposure_state() {
    let root = temp_dir("catalog-skills");
    let registry = ExtensionRegistryBuilder::new().build().expect("registry");
    let skills = vec![
        skill_descriptor(
            "commit",
            "roder-builtin://commit/SKILL.md",
            SkillSource::BuiltIn,
            SkillActivationState::Enabled,
            SkillExposure::DirectOnly,
        ),
        skill_descriptor(
            "review",
            "workspace://.agents/skills/review/SKILL.md",
            SkillSource::Workspace,
            SkillActivationState::Disabled,
            SkillExposure::Global,
        ),
    ];

    let result = build_file_backed_catalog_with_skills(
        &registry,
        &[],
        &skills,
        &DiscoveryCatalogBuildOptions::new(root.join("catalog"), root.join("session")),
    )
    .expect("build catalog");

    let group = result
        .catalog
        .groups
        .iter()
        .find(|group| group.id == "skills:registry")
        .expect("skills group");
    assert_eq!(group.item_count, 2);
    let commit = group
        .items
        .iter()
        .find(|item| item.name == "commit")
        .expect("commit skill");
    assert_eq!(commit.source.kind, DiscoverySourceKind::Skills);
    assert_eq!(commit.status, DiscoveryItemStatus::Available);
    assert!(commit.tags.contains(&"built-in".to_string()));
    assert!(commit.tags.contains(&"direct-only".to_string()));
    let review = group
        .items
        .iter()
        .find(|item| item.name == "review")
        .expect("review skill");
    assert_eq!(review.status, DiscoveryItemStatus::Disabled);
    assert!(review.tags.contains(&"global".to_string()));
    assert!(
        root.join("catalog")
            .join(commit.schema.as_ref().unwrap().uri.as_str())
            .exists()
    );
}

fn workflow_item(
    name: &str,
    source_type: WorkflowSourceType,
    command_capable: bool,
    preview: serde_json::Value,
) -> WorkflowImportItem {
    WorkflowImportItem {
        id: name.to_string(),
        title: name.to_string(),
        summary: "workflow summary".to_string(),
        source: WorkflowSource {
            source_type,
            path: format!("{name}.json"),
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
        preview,
        conflicts: Vec::new(),
        enabled_at: None,
    }
}

fn skill_descriptor(
    name: &str,
    path: &str,
    source: SkillSource,
    activation: SkillActivationState,
    exposure: SkillExposure,
) -> SkillDescriptor {
    SkillDescriptor {
        id: path.to_string(),
        name: name.to_string(),
        canonical_path: path.to_string(),
        source,
        exposure,
        activation,
        description: format!("{name} skill"),
        short_description: None,
        experimental: false,
        diagnostics: Vec::new(),
        agent_metadata: None,
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "roder-discovery-{name}-{}",
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp dir");
    root
}
