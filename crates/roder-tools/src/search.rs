use std::sync::Arc;

use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use roder_search::{DEFAULT_MAX_FILE_SIZE, SearchMode, SearchOptions};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::backend::{WorkspaceBackendHandle, backend_from_context_or_fallback};
use crate::files::{parse, require_nonempty, result};
use crate::paging::{
    DEFAULT_PAGE_LINES, MAX_PAGE_LINES, append_continuation_instruction, clamp_limit, page_lines,
    page_metadata_with_continuation,
};
use crate::workspace::Workspace;

pub(crate) fn register(
    registry: &mut ToolRegistry,
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
) -> anyhow::Result<()> {
    registry.register(Arc::new(GrepTool {
        workspace: workspace.clone(),
        backend: backend.clone(),
    }))?;
    registry.register(Arc::new(GlobTool { workspace, backend }))
}

struct GrepTool {
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
}

#[async_trait::async_trait]
impl ToolExecutor for GrepTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "grep".to_string(),
            description: "Search text files for a literal or regex query with paginated output. Relative paths resolve from the workspace root."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "path": { "type": "string", "default": "." },
                    "regex": {
                        "type": "boolean",
                        "default": false,
                        "description": "Treat query as a regular expression instead of a literal string."
                    },
                    "case_sensitive": {
                        "type": "boolean",
                        "default": true,
                        "description": "Match case exactly when true."
                    },
                    "word_boundary": {
                        "type": "boolean",
                        "default": false,
                        "description": "Require word boundaries around the query."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["auto", "indexed", "scan"],
                        "default": "auto",
                        "description": "Search engine preference. Auto uses the index when it can narrow candidates."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "default": 0,
                        "description": "Zero-based match offset for pagination."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_PAGE_LINES,
                        "default": DEFAULT_PAGE_LINES,
                        "description": "Maximum number of matching lines to return."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let args = parse::<GrepArgs>(&call)?;
        require_nonempty(&args.query, "query")?;
        let offset = args.offset.unwrap_or_default();
        let limit = clamp_limit(args.limit);
        let query = args.query.clone();
        let path = args.path.clone().unwrap_or_else(|| ".".to_string());
        let regex = args.regex.unwrap_or(false);
        let case_sensitive = args.case_sensitive.unwrap_or(true);
        let word_boundary = args.word_boundary.unwrap_or(false);
        let mode = args.mode.clone().unwrap_or_else(|| "auto".to_string());
        let backend = backend_from_context_or_fallback(&ctx, &self.workspace, &self.backend)?;
        let options = args.into_search_options()?;
        let (start, matches, metadata) = backend.grep_search(options).await?;
        let page = page_lines(&matches, offset, limit);
        let continuation_args = page.next_offset.map(|next| {
            json!({
                "query": query,
                "path": path,
                "regex": regex,
                "case_sensitive": case_sensitive,
                "word_boundary": word_boundary,
                "mode": mode,
                "offset": next,
                "limit": limit,
            })
        });
        let mut text = page.text.clone();
        if let Some(args) = continuation_args.as_ref() {
            append_continuation_instruction(&mut text, &page, "grep", args);
        }
        let mut data = page_metadata_with_continuation(
            start,
            offset,
            limit,
            &page,
            "grep",
            continuation_args.unwrap_or(Value::Null),
        );
        merge_search_metadata(&mut data, &metadata);
        Ok(result(call, text, data, false))
    }
}

struct GlobTool {
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
}

#[async_trait::async_trait]
impl ToolExecutor for GlobTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "glob".to_string(),
            description:
                "Find files under the workspace root matching a glob pattern with paginated output."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "default": 0,
                        "description": "Zero-based match offset for pagination."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_PAGE_LINES,
                        "default": DEFAULT_PAGE_LINES,
                        "description": "Maximum number of file paths to return."
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let args = parse::<GlobArgs>(&call)?;
        require_nonempty(&args.pattern, "pattern")?;
        let backend = backend_from_context_or_fallback(&ctx, &self.workspace, &self.backend)?;
        let matches = backend.glob(&args.pattern).await?;
        let offset = args.offset.unwrap_or_default();
        let limit = clamp_limit(args.limit);
        let page = page_lines(&matches, offset, limit);
        let continuation_args = page.next_offset.map(|next| {
            json!({
                "pattern": args.pattern,
                "offset": next,
                "limit": limit,
            })
        });
        let mut text = page.text.clone();
        if let Some(args) = continuation_args.as_ref() {
            append_continuation_instruction(&mut text, &page, "glob", args);
        }
        let data = page_metadata_with_continuation(
            ".".to_string(),
            offset,
            limit,
            &page,
            "glob",
            continuation_args.unwrap_or(Value::Null),
        );
        Ok(result(call, text, data, false))
    }
}

#[derive(Deserialize)]
struct GrepArgs {
    query: String,
    path: Option<String>,
    regex: Option<bool>,
    case_sensitive: Option<bool>,
    word_boundary: Option<bool>,
    mode: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
}

impl GrepArgs {
    fn into_search_options(self) -> anyhow::Result<SearchOptions> {
        let mode = match self.mode.as_deref().unwrap_or("auto") {
            "auto" => SearchMode::Auto,
            "indexed" => SearchMode::Indexed,
            "scan" => SearchMode::Scan,
            other => anyhow::bail!("unsupported grep mode: {other}"),
        };
        Ok(SearchOptions {
            query: self.query,
            path: self.path.unwrap_or_else(|| ".".to_string()).into(),
            mode,
            regex: self.regex.unwrap_or(false),
            case_sensitive: self.case_sensitive.unwrap_or(true),
            word_boundary: self.word_boundary.unwrap_or(false),
            max_file_size: DEFAULT_MAX_FILE_SIZE,
        })
    }
}

#[derive(Deserialize)]
struct GlobArgs {
    pattern: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

pub(crate) fn visit_files(
    root: &std::path::Path,
    visitor: &mut dyn FnMut(&std::path::Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if root.is_file() {
        return visitor(root);
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if entry.file_type()?.is_dir() {
            if file_name == ".git" || file_name == "target" {
                continue;
            }
            visit_files(&path, visitor)?;
        } else {
            visitor(&path)?;
        }
    }
    Ok(())
}

fn merge_search_metadata(data: &mut Value, metadata: &roder_search::SearchMetadata) {
    let Some(object) = data.as_object_mut() else {
        return;
    };
    object.insert("engine".to_string(), json!(metadata.engine.as_str()));
    object.insert("index_version".to_string(), json!(metadata.index_version));
    object.insert(
        "candidate_files".to_string(),
        json!(metadata.candidate_files),
    );
    object.insert("verified_files".to_string(), json!(metadata.verified_files));
    object.insert("stale".to_string(), json!(metadata.stale));
    object.insert("elapsed_ms".to_string(), json!(metadata.elapsed_ms));
    object.insert("index_bytes".to_string(), json!(metadata.index_bytes));
    object.insert(
        "index_build_time_ms".to_string(),
        json!(metadata.index_build_time_ms),
    );
}

pub(crate) fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let (mut p, mut t) = (0, 0);
    let mut star = None;
    let mut star_text = 0;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            star_text = t;
        } else if let Some(star_index) = star {
            p = star_index + 1;
            star_text += 1;
            t = star_text;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::LocalWorkspaceBackend;
    use roder_api::tools::{LocalWorkspaceHandle, ToolExecutionContext};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    #[test]
    fn wildcard_match_supports_star_and_question_mark() {
        assert!(wildcard_match("src/*.rs", "src/main.rs"));
        assert!(wildcard_match("src/??.rs", "src/io.rs"));
        assert!(!wildcard_match("src/*.rs", "README.md"));
    }

    #[tokio::test]
    async fn grep_paging_result_includes_continuation_text_and_data() {
        let root = test_workspace("grep-paging");
        let dir = root.join("src");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "needle a\n").unwrap();
        std::fs::write(dir.join("b.rs"), "needle b\n").unwrap();
        let workspace = Workspace::new(root.clone()).unwrap();
        let tool = GrepTool {
            workspace: workspace.clone(),
            backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
        };

        let result = tool
            .execute(
                context(&root),
                call(
                    "grep",
                    json!({"query": "needle", "path": "src", "limit": 1}),
                ),
            )
            .await
            .unwrap();

        assert!(result.text.contains("call grep"));
        assert!(result.text.contains("\"offset\":1"));
        assert_eq!(result.data["omitted_lines"], 1);
        assert_eq!(result.data["continuation_tool"], "grep");
        assert_eq!(result.data["continuation_args"]["query"], "needle");
        assert_eq!(result.data["continuation_args"]["offset"], 1);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn glob_paging_result_includes_continuation_text_and_data() {
        let root = test_workspace("glob-paging");
        let dir = root.join("src");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "").unwrap();
        std::fs::write(dir.join("b.rs"), "").unwrap();
        let workspace = Workspace::new(root.clone()).unwrap();
        let tool = GlobTool {
            workspace: workspace.clone(),
            backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
        };

        let result = tool
            .execute(
                context(&root),
                call("glob", json!({"pattern": "src/*.rs", "limit": 1})),
            )
            .await
            .unwrap();

        assert!(result.text.contains("call glob"));
        assert!(result.text.contains("\"offset\":1"));
        assert_eq!(result.data["omitted_lines"], 1);
        assert_eq!(result.data["continuation_tool"], "glob");
        assert_eq!(result.data["continuation_args"]["pattern"], "src/*.rs");
        assert_eq!(result.data["continuation_args"]["offset"], 1);

        let _ = std::fs::remove_dir_all(root);
    }

    fn context(workspace: &Path) -> ToolExecutionContext {
        ToolExecutionContext::new(
            "thread-a",
            "turn-a",
            roder_api::policy_mode::PolicyMode::Default,
        )
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(workspace)))
    }

    fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            raw_arguments: arguments.to_string(),
            arguments,
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
        }
    }

    fn test_workspace(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("roder-tools-{name}-{stamp}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
