use std::sync::Arc;

use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::files::{parse, require_nonempty, result};
use crate::paging::{DEFAULT_PAGE_LINES, MAX_PAGE_LINES, clamp_limit};

pub(crate) fn register(registry: &mut ToolRegistry) -> anyhow::Result<()> {
    registry.register(Arc::new(ReadArtifactTool))?;
    registry.register(Arc::new(GrepArtifactTool))?;
    registry.register(Arc::new(TailArtifactTool))
}

struct ReadArtifactTool;

#[async_trait::async_trait]
impl ToolExecutor for ReadArtifactTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_artifact".to_string(),
            description: "Read a Roder context artifact for the current thread by artifact id, optionally by line range."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "artifact_id": { "type": "string" },
                    "start_line": { "type": "integer", "minimum": 1, "default": 1 },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_PAGE_LINES,
                        "default": DEFAULT_PAGE_LINES
                    }
                },
                "required": ["artifact_id"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<ReadArtifactArgs>(&call)?;
        require_nonempty(&args.artifact_id, "artifact_id")?;
        let store = ctx.require_context_artifacts()?;
        let page = store.read_artifact(
            &ctx.thread_id,
            &args.artifact_id,
            args.start_line.unwrap_or(1),
            clamp_limit(args.limit),
        )?;
        let text = page.text.clone();
        Ok(result(call, text, serde_json::to_value(page)?, false))
    }
}

struct GrepArtifactTool;

#[async_trait::async_trait]
impl ToolExecutor for GrepArtifactTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "grep_artifact".to_string(),
            description: "Search a Roder context artifact for the current thread by literal query."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "artifact_id": { "type": "string" },
                    "query": { "type": "string" },
                    "offset": { "type": "integer", "minimum": 0, "default": 0 },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_PAGE_LINES,
                        "default": DEFAULT_PAGE_LINES
                    }
                },
                "required": ["artifact_id", "query"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<GrepArtifactArgs>(&call)?;
        require_nonempty(&args.artifact_id, "artifact_id")?;
        require_nonempty(&args.query, "query")?;
        let store = ctx.require_context_artifacts()?;
        let page = store.grep_artifact(
            &ctx.thread_id,
            &args.artifact_id,
            &args.query,
            args.offset.unwrap_or_default(),
            clamp_limit(args.limit),
        )?;
        let text = page.text.clone();
        Ok(result(call, text, serde_json::to_value(page)?, false))
    }
}

struct TailArtifactTool;

#[async_trait::async_trait]
impl ToolExecutor for TailArtifactTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "tail_artifact".to_string(),
            description: "Read the final lines of a Roder context artifact for the current thread."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "artifact_id": { "type": "string" },
                    "lines": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_PAGE_LINES,
                        "default": DEFAULT_PAGE_LINES
                    }
                },
                "required": ["artifact_id"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<TailArtifactArgs>(&call)?;
        require_nonempty(&args.artifact_id, "artifact_id")?;
        let store = ctx.require_context_artifacts()?;
        let page =
            store.tail_artifact(&ctx.thread_id, &args.artifact_id, clamp_limit(args.lines))?;
        let text = page.text.clone();
        Ok(result(call, text, serde_json::to_value(page)?, false))
    }
}

#[derive(Deserialize)]
struct ReadArtifactArgs {
    artifact_id: String,
    start_line: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct GrepArtifactArgs {
    artifact_id: String,
    query: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct TailArtifactArgs {
    artifact_id: String,
    lines: Option<usize>,
}
