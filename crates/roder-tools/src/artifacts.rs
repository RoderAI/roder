use std::path::PathBuf;
use std::sync::Arc;

use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use roder_core::artifacts::{ContextArtifactStore, default_sessions_dir, session_artifact_dir};
use serde::Deserialize;
use serde_json::json;

use crate::files::{parse, require_nonempty, result};
use crate::paging::{DEFAULT_PAGE_LINES, MAX_PAGE_LINES, clamp_limit};

pub(crate) fn register(registry: &mut ToolRegistry) -> anyhow::Result<()> {
    registry.register(Arc::new(ReadArtifactTool))?;
    registry.register(Arc::new(GrepArtifactTool))
}

#[derive(Debug, Default)]
pub struct ArtifactToolsContributor;

impl roder_api::tools::ToolContributor for ArtifactToolsContributor {
    fn id(&self) -> roder_api::extension::ToolProviderId {
        "builtin-artifact-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        register(registry)
    }
}

fn store_for(ctx: &ToolExecutionContext) -> ContextArtifactStore {
    let root = ctx
        .context_artifact_dir
        .clone()
        .or_else(|| std::env::var_os("RODER_SESSION_DIR").map(PathBuf::from))
        .unwrap_or_else(|| {
            let sessions = default_sessions_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("roder-sessions"));
            session_artifact_dir(sessions, &ctx.thread_id)
        });
    ContextArtifactStore::new(root)
}

#[derive(Debug)]
struct ReadArtifactTool;

#[derive(Debug)]
struct GrepArtifactTool;

#[derive(Debug, Deserialize)]
struct ReadArtifactArgs {
    artifact_id: String,
    #[serde(default)]
    start_line: Option<u64>,
    #[serde(default)]
    limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GrepArtifactArgs {
    artifact_id: String,
    pattern: String,
}

#[async_trait::async_trait]
impl ToolExecutor for ReadArtifactTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_artifact".to_string(),
            description: "Read a paginated slice of a file-backed context artifact by id. Scoped to the current thread."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "artifact_id": { "type": "string" },
                    "start_line": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "One-based line to start reading from."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_PAGE_LINES,
                        "default": DEFAULT_PAGE_LINES,
                        "description": "Maximum number of lines to return. Use next_start_line from the response to continue."
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
        let start_line = args.start_line.unwrap_or(1).max(1);
        let limit = clamp_limit(args.limit.map(|value| value as usize)) as u64;
        let store = store_for(&ctx);
        let page = store.read_page(
            &ctx.thread_id,
            &args.artifact_id,
            start_line,
            limit,
        )?;
        let numbered: Vec<String> = page
            .text
            .lines()
            .enumerate()
            .map(|(index, line)| {
                format!(
                    "{:>5}: {}",
                    page.start_line.saturating_add(index as u64),
                    line
                )
            })
            .collect();
        let mut text = numbered.join("\n");
        if let Some(next) = page.next_start_line {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!(
                "[showing lines {}-{} of {}; next_start_line={next}]",
                page.start_line,
                page.start_line.saturating_add(page.line_count.saturating_sub(1)),
                page.total_lines
            ));
        }
        Ok(result(
            call,
            text,
            json!({
                "artifact_id": page.artifact_id,
                "start_line": page.start_line,
                "line_count": page.line_count,
                "total_lines": page.total_lines,
                "next_start_line": page.next_start_line,
                "truncated": page.next_start_line.is_some(),
            }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for GrepArtifactTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "grep_artifact".to_string(),
            description: "Search a file-backed context artifact for a literal pattern. Scoped to the current thread."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "artifact_id": { "type": "string" },
                    "pattern": { "type": "string" }
                },
                "required": ["artifact_id", "pattern"],
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
        require_nonempty(&args.pattern, "pattern")?;
        let store = store_for(&ctx);
        let grep = store.grep(&ctx.thread_id, &args.artifact_id, &args.pattern)?;
        let mut lines: Vec<String> = grep
            .matches
            .iter()
            .map(|m| format!("{}:{}", m.line_number, m.line))
            .collect();
        if grep.truncated {
            lines.push(format!(
                "[truncated after {} matches; refine pattern]",
                grep.matches.len()
            ));
        }
        let text = if lines.is_empty() {
            format!("no matches for {:?} in {}", args.pattern, args.artifact_id)
        } else {
            lines.join("\n")
        };
        Ok(result(
            call,
            text,
            json!({
                "artifact_id": grep.artifact_id,
                "pattern": grep.pattern,
                "match_count": grep.matches.len(),
                "truncated": grep.truncated,
            }),
            false,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::artifacts::ContextArtifactKind;
    use roder_api::policy_mode::PolicyMode;
    use roder_api::tools::{ToolCall, ToolContributor, ToolRegistry};
    use serde_json::json;

    #[tokio::test]
    async fn read_and_grep_artifact_tools_page_and_search() {
        let root = std::env::temp_dir().join(format!(
            "roder-tools-artifacts-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ContextArtifactStore::new(&root);
        let body = (1..=30)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        store
            .write(
                "thread-a",
                "turn-b",
                ContextArtifactKind::ToolOutput,
                "call_99",
                None,
                "stdout",
                body.as_bytes(),
            )
            .unwrap();

        let mut registry = ToolRegistry::default();
        ArtifactToolsContributor.contribute(&mut registry).unwrap();

        let ctx = ToolExecutionContext::new("thread-a", "turn-b", PolicyMode::Default)
            .with_context_artifact_dir(&root);

        let read = registry
            .get("read_artifact")
            .unwrap()
            .execute(
                ctx.clone(),
                call(
                    "read_artifact",
                    json!({ "artifact_id": "call_99", "start_line": 1, "limit": 5 }),
                ),
            )
            .await
            .unwrap();
        assert!(read.text.contains("line 1"));
        assert_eq!(read.data["next_start_line"], 6);

        let grep = registry
            .get("grep_artifact")
            .unwrap()
            .execute(
                ctx,
                call(
                    "grep_artifact",
                    json!({ "artifact_id": "call_99", "pattern": "line 17" }),
                ),
            )
            .await
            .unwrap();
        assert!(grep.text.contains("17:line 17"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn read_artifact_rejects_cross_thread_access() {
        let root = std::env::temp_dir().join(format!(
            "roder-tools-artifacts-xthread-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ContextArtifactStore::new(&root);
        store
            .write(
                "thread-a",
                "turn-b",
                ContextArtifactKind::ToolOutput,
                "call_1",
                None,
                "stdout",
                b"secret\n",
            )
            .unwrap();

        let mut registry = ToolRegistry::default();
        ArtifactToolsContributor.contribute(&mut registry).unwrap();
        let ctx = ToolExecutionContext::new("thread-b", "turn-b", PolicyMode::Default)
            .with_context_artifact_dir(&root);

        let err = registry
            .get("read_artifact")
            .unwrap()
            .execute(
                ctx,
                call("read_artifact", json!({ "artifact_id": "call_1" })),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("thread-a"));

        let _ = std::fs::remove_dir_all(root);
    }

    fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            raw_arguments: arguments.to_string(),
            arguments,
            thread_id: "thread-a".to_string(),
            turn_id: "turn-b".to_string(),
        }
    }
}
