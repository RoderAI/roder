use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::artifacts::ContextArtifactAccess;
use crate::discovery::{
    DiscoveryAuthState, DiscoveryCacheStatus, DiscoveryCatalogItem, DiscoveryCatalogSource,
    DiscoveryItemStatus, DiscoveryLifecycleState, DiscoveryPromotionState, DiscoveryRedaction,
    DiscoverySchemaFormat, DiscoverySchemaReference, DiscoverySourceKind,
};
use crate::events::{ThreadId, TurnId};
use crate::extension::ToolProviderId;
use crate::inference::ModelSchemaPolicy;
use crate::media::{MediaGenerationRequest, MediaGenerationResponse};
use crate::policy_mode::PolicyMode;
use crate::trace::SubagentTraceSink;
use crate::{ToolSchemaPolicy, normalize_tool_schema};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    pub fn normalized_for_model(&self, policy: ToolSchemaPolicy) -> Self {
        let mut spec = self.clone();
        spec.parameters = normalize_tool_schema(&spec.name, &spec.parameters, policy).schema;
        spec
    }

    pub fn normalized_for_model_profile(&self, policy: ModelSchemaPolicy) -> Self {
        match policy {
            ModelSchemaPolicy::StandardRequiredFirst => {
                self.normalized_for_model(ToolSchemaPolicy::warning())
            }
            ModelSchemaPolicy::RequiredFirstFlat => {
                self.normalized_for_model(ToolSchemaPolicy::strict())
            }
        }
    }

    pub fn discovery_item(
        &self,
        provider_id: impl Into<String>,
        schema_uri: impl Into<String>,
    ) -> DiscoveryCatalogItem {
        let provider_id = provider_id.into();
        DiscoveryCatalogItem {
            id: format!("tool:{provider_id}/{}", self.name),
            group_id: format!("tools:{provider_id}"),
            source: DiscoveryCatalogSource {
                kind: DiscoverySourceKind::InternalTools,
                id: provider_id.clone(),
                display_name: provider_id,
                origin: None,
                auth_state: DiscoveryAuthState::NotRequired,
                redaction: DiscoveryRedaction::none(),
            },
            name: self.name.clone(),
            title: self.name.clone(),
            description: Some(self.description.clone()),
            status: DiscoveryItemStatus::Available,
            lifecycle: DiscoveryLifecycleState::Discovered,
            promotion: DiscoveryPromotionState::NotPromoted,
            cache_status: DiscoveryCacheStatus::Cold,
            schema: Some(DiscoverySchemaReference {
                format: DiscoverySchemaFormat::JsonSchema,
                uri: schema_uri.into(),
                content_hash: None,
                byte_count: None,
                redaction: DiscoveryRedaction::none(),
            }),
            tags: vec!["tool".to_string()],
            hints: Vec::new(),
            redaction: DiscoveryRedaction::none(),
            last_refreshed_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolChoice {
    Auto,
    Any,
    None,
    Specific(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub raw_arguments: String,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub id: String,
    pub name: String,
    pub text: String,
    pub data: serde_json::Value,
    pub is_error: bool,
}

#[derive(Clone, Default)]
pub struct ToolExecutionHandles {
    pub workspace: Option<Arc<dyn ScopedWorkspaceHandle>>,
    pub process_runner: Option<Arc<dyn ScopedProcessRunner>>,
    pub subagent_trace_sink: Option<Arc<dyn SubagentTraceSink>>,
    pub context_artifacts: Option<Arc<dyn ContextArtifactAccess>>,
}

impl fmt::Debug for ToolExecutionHandles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolExecutionHandles")
            .field("workspace", &self.workspace.is_some())
            .field("process_runner", &self.process_runner.is_some())
            .field("subagent_trace_sink", &self.subagent_trace_sink.is_some())
            .field("context_artifacts", &self.context_artifacts.is_some())
            .finish()
    }
}

pub trait ScopedWorkspaceHandle: Send + Sync + 'static {
    fn workspace_root(&self) -> Option<PathBuf>;
}

pub trait ScopedProcessRunner: Send + Sync + 'static {
    fn runner_name(&self) -> &str;
}

#[derive(Debug, Clone)]
pub struct LocalWorkspaceHandle {
    root: PathBuf,
}

impl LocalWorkspaceHandle {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl ScopedWorkspaceHandle for LocalWorkspaceHandle {
    fn workspace_root(&self) -> Option<PathBuf> {
        Some(self.root.clone())
    }
}

#[derive(Debug, Clone, Default)]
pub struct LocalProcessRunnerHandle;

impl ScopedProcessRunner for LocalProcessRunnerHandle {
    fn runner_name(&self) -> &str {
        "local-process"
    }
}

#[derive(Debug, Clone)]
pub struct ToolExecutionContext {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub effective_mode: PolicyMode,
    pub handles: ToolExecutionHandles,
}

impl ToolExecutionContext {
    pub fn new(
        thread_id: impl Into<ThreadId>,
        turn_id: impl Into<TurnId>,
        effective_mode: PolicyMode,
    ) -> Self {
        Self {
            thread_id: thread_id.into(),
            turn_id: turn_id.into(),
            effective_mode,
            handles: ToolExecutionHandles::default(),
        }
    }

    pub fn with_workspace_handle(mut self, handle: Arc<dyn ScopedWorkspaceHandle>) -> Self {
        self.handles.workspace = Some(handle);
        self
    }

    pub fn with_process_runner(mut self, runner: Arc<dyn ScopedProcessRunner>) -> Self {
        self.handles.process_runner = Some(runner);
        self
    }

    pub fn with_subagent_trace_sink(mut self, sink: Arc<dyn SubagentTraceSink>) -> Self {
        self.handles.subagent_trace_sink = Some(sink);
        self
    }

    pub fn with_context_artifacts(mut self, store: Arc<dyn ContextArtifactAccess>) -> Self {
        self.handles.context_artifacts = Some(store);
        self
    }

    pub fn require_workspace(&self) -> anyhow::Result<Arc<dyn ScopedWorkspaceHandle>> {
        self.handles
            .workspace
            .clone()
            .ok_or_else(|| anyhow::anyhow!("workspace handle is not available"))
    }

    pub fn require_process_runner(&self) -> anyhow::Result<Arc<dyn ScopedProcessRunner>> {
        self.handles
            .process_runner
            .clone()
            .ok_or_else(|| anyhow::anyhow!("process runner is not available"))
    }

    pub fn require_context_artifacts(&self) -> anyhow::Result<Arc<dyn ContextArtifactAccess>> {
        self.handles
            .context_artifacts
            .clone()
            .ok_or_else(|| anyhow::anyhow!("context artifact store is not available"))
    }
}

#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync + 'static {
    fn spec(&self) -> ToolSpec;

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult>;
}

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn ToolExecutor>>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: Arc<dyn ToolExecutor>) -> anyhow::Result<()> {
        let name = tool.spec().name;
        if self.tools.contains_key(&name) {
            anyhow::bail!("tool {name:?} is already registered");
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .values()
            .map(|tool| {
                tool.spec()
                    .normalized_for_model(ToolSchemaPolicy::warning())
            })
            .collect()
    }

    pub fn specs_for_edit_tool(&self, edit_tool: Option<&str>) -> Vec<ToolSpec> {
        self.specs_for_edit_tool_with_schema_policy(edit_tool, ModelSchemaPolicy::RequiredFirstFlat)
    }

    pub fn specs_for_edit_tool_with_schema_policy(
        &self,
        edit_tool: Option<&str>,
        schema_policy: ModelSchemaPolicy,
    ) -> Vec<ToolSpec> {
        self.tools
            .values()
            .map(|tool| tool.spec())
            .filter(|spec| keep_tool_for_edit_tool(&spec.name, edit_tool))
            .map(|spec| spec.normalized_for_model_profile(schema_policy))
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ToolExecutor>> {
        self.tools.get(name).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

fn keep_tool_for_edit_tool(name: &str, edit_tool: Option<&str>) -> bool {
    match name {
        "apply_patch" => matches!(edit_tool, Some("patch")),
        "write_file" | "edit" | "multi_edit" => !matches!(edit_tool, Some("patch")),
        _ => true,
    }
}

pub trait ToolContributor: Send + Sync + 'static {
    fn id(&self) -> ToolProviderId;
    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait MediaGeneratorProvider: Send + Sync + 'static {
    fn provider_id(&self) -> &str;
    fn supports_images(&self) -> bool;
    fn supports_videos(&self) -> bool;

    async fn generate_image(
        &self,
        _request: MediaGenerationRequest,
    ) -> anyhow::Result<MediaGenerationResponse> {
        anyhow::bail!("image generation is not supported by this provider")
    }

    async fn generate_video(
        &self,
        _request: MediaGenerationRequest,
    ) -> anyhow::Result<MediaGenerationResponse> {
        anyhow::bail!("video generation is not supported by this provider")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_spec_can_be_represented_as_discovery_item() {
        let spec = ToolSpec {
            name: "grep".to_string(),
            description: "Search files".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }),
        };

        let item = spec.discovery_item(
            "builtin-coding-tools",
            "discovery/tools/builtin-coding-tools/grep.schema.json",
        );
        assert_eq!(item.id, "tool:builtin-coding-tools/grep");
        assert_eq!(item.group_id, "tools:builtin-coding-tools");
        assert_eq!(item.source.kind, DiscoverySourceKind::InternalTools);
        assert_eq!(item.source.auth_state, DiscoveryAuthState::NotRequired);
        assert_eq!(item.status, DiscoveryItemStatus::Available);
        assert_eq!(item.lifecycle, DiscoveryLifecycleState::Discovered);
        assert_eq!(
            item.schema.as_ref().map(|schema| schema.format.clone()),
            Some(DiscoverySchemaFormat::JsonSchema)
        );
    }
}
