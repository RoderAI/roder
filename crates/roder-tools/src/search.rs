use std::path::Path;
use std::sync::Arc;

use ignore::WalkBuilder;
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
use crate::response_format::ResponseFormat;
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
            description: "Search text files for a literal or regex query with paginated output. Relative paths resolve from the workspace root; absolute paths are searched directly."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "path": {
                        "type": "string",
                        "default": ".",
                        "description": "Directory or file to search. Relative paths resolve from the workspace root; an absolute path is searched directly."
                    },
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
                    },
                    "response_format": ResponseFormat::schema_property()
                },
                "required": ["query"],
                "additionalProperties": false,
                "x-roder": {
                    "retrievalMode": "exact_text",
                    "retrievalMetadata": true
                }
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
        let response_format = args.response_format.unwrap_or_default();
        let backend = backend_from_context_or_fallback(&ctx, &self.workspace, &self.backend)?;
        let options = args.into_search_options()?;
        let (start, matches, metadata) = backend.grep_search(options).await?;
        let matches = matches
            .iter()
            .map(|line| response_format.format_line(line))
            .collect::<Vec<_>>();
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
                "response_format": response_format.as_str(),
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
        data["response_format"] = json!(response_format.as_str());
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
                    },
                    "response_format": ResponseFormat::schema_property()
                },
                "required": ["pattern"],
                "additionalProperties": false,
                "x-roder": {
                    "retrievalMode": "file_name",
                    "retrievalMetadata": true
                }
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
        let response_format = args.response_format.unwrap_or_default();
        let page = page_lines(&matches, offset, limit);
        let continuation_args = page.next_offset.map(|next| {
            json!({
                "pattern": args.pattern,
                "offset": next,
                "limit": limit,
                "response_format": response_format.as_str(),
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
        let mut data = data;
        data["response_format"] = json!(response_format.as_str());
        data["retrieval_mode"] = json!("file_name");
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
    response_format: Option<ResponseFormat>,
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
    response_format: Option<ResponseFormat>,
}

pub(crate) fn visit_files(
    root: &Path,
    visitor: &mut dyn FnMut(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if root.is_file() {
        return visitor(root);
    }
    let mut walk = WalkBuilder::new(root);
    walk.standard_filters(true)
        .hidden(false)
        .require_git(false)
        .filter_entry({
            let root = root.to_path_buf();
            move |entry| !ignored_path(&root, entry.path())
        })
        .sort_by_file_path(|left, right| left.cmp(right));

    for entry in walk.build() {
        let entry = entry?;
        let path = entry.path();
        if path == root {
            continue;
        }
        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            visitor(path)?;
        }
    }
    Ok(())
}

fn ignored_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    relative.components().any(|component| {
        let value = component.as_os_str().to_string_lossy();
        matches!(value.as_ref(), ".git" | ".roder" | "target")
    })
}

fn merge_search_metadata(data: &mut Value, metadata: &roder_search::SearchMetadata) {
    let Some(object) = data.as_object_mut() else {
        return;
    };
    object.insert("engine".to_string(), json!(metadata.engine.as_str()));
    object.insert("retrieval_mode".to_string(), json!("exact_text"));
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

pub(crate) fn normalize_relative_pattern(pattern: &str) -> String {
    let mut parts = Vec::new();
    let pattern = pattern.replace('\\', "/");
    for part in pattern.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    if parts.is_empty() {
        "*".to_string()
    } else {
        parts.join("/")
    }
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
