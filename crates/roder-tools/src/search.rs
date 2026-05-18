use std::sync::Arc;

use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::backend::WorkspaceBackendHandle;
use crate::files::{parse, require_nonempty, result};
use crate::paging::{DEFAULT_PAGE_LINES, MAX_PAGE_LINES, clamp_limit, page_lines, page_metadata};

pub(crate) fn register(
    registry: &mut ToolRegistry,
    backend: WorkspaceBackendHandle,
) -> anyhow::Result<()> {
    registry.register(Arc::new(GrepTool {
        backend: backend.clone(),
    }))?;
    registry.register(Arc::new(GlobTool { backend }))
}

struct GrepTool {
    backend: WorkspaceBackendHandle,
}

#[async_trait::async_trait]
impl ToolExecutor for GrepTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "grep".to_string(),
            description: "Search workspace text files for a literal query with paginated output."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "path": { "type": "string", "default": "." },
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
        let (start, matches) = self
            .backend
            .grep_literal(&args.query, args.path.as_deref().unwrap_or("."))
            .await?;
        let offset = args.offset.unwrap_or_default();
        let limit = clamp_limit(args.limit);
        let page = page_lines(&matches, offset, limit);
        let data = page_metadata(start, offset, limit, &page);
        Ok(result(call, page.text, data, false))
    }
}

struct GlobTool {
    backend: WorkspaceBackendHandle,
}

#[async_trait::async_trait]
impl ToolExecutor for GlobTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "glob".to_string(),
            description: "Find workspace files matching a glob pattern with paginated output."
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
        let matches = self.backend.glob(&args.pattern).await?;
        let offset = args.offset.unwrap_or_default();
        let limit = clamp_limit(args.limit);
        let page = page_lines(&matches, offset, limit);
        let data = page_metadata(".".to_string(), offset, limit, &page);
        Ok(result(call, page.text, data, false))
    }
}

#[derive(Deserialize)]
struct GrepArgs {
    query: String,
    path: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
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

    #[test]
    fn wildcard_match_supports_star_and_question_mark() {
        assert!(wildcard_match("src/*.rs", "src/main.rs"));
        assert!(wildcard_match("src/??.rs", "src/io.rs"));
        assert!(!wildcard_match("src/*.rs", "README.md"));
    }
}
