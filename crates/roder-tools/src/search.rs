use std::sync::Arc;

use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::files::{parse, require_nonempty, result};
use crate::paging::{DEFAULT_PAGE_LINES, MAX_PAGE_LINES, clamp_limit, page_lines, page_metadata};
use crate::workspace::Workspace;

pub(crate) fn register(registry: &mut ToolRegistry, workspace: Workspace) -> anyhow::Result<()> {
    registry.register(Arc::new(GrepTool {
        workspace: workspace.clone(),
    }))?;
    registry.register(Arc::new(GlobTool { workspace }))
}

#[derive(Debug)]
struct GrepTool {
    workspace: Workspace,
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
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<GrepArgs>(&call)?;
        require_nonempty(&args.query, "query")?;
        let start = self
            .workspace
            .resolve_existing(args.path.as_deref().unwrap_or("."))?;
        let mut matches = Vec::new();
        let offset = args.offset.unwrap_or_default();
        let limit = clamp_limit(args.limit);
        let collect_until = offset.saturating_add(limit).saturating_add(1);
        visit_files(&start, &mut |path| {
            if matches.len() >= collect_until {
                return Ok(());
            }
            let Ok(text) = std::fs::read_to_string(path) else {
                return Ok(());
            };
            for (line_index, line) in text.lines().enumerate() {
                if line.contains(&args.query) {
                    matches.push(format!(
                        "{}:{}:{}",
                        self.workspace.display(path),
                        line_index + 1,
                        line
                    ));
                    if matches.len() >= collect_until {
                        break;
                    }
                }
            }
            Ok(())
        })?;
        let page = page_lines(&matches, offset, limit);
        let data = page_metadata(self.workspace.display(&start), offset, limit, &page);
        Ok(result(call, page.text, data, false))
    }
}

#[derive(Debug)]
struct GlobTool {
    workspace: Workspace,
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
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<GlobArgs>(&call)?;
        require_nonempty(&args.pattern, "pattern")?;
        let mut matches = Vec::new();
        visit_files(self.workspace.root(), &mut |path| {
            let rel = self.workspace.display(path);
            if wildcard_match(&args.pattern, &rel) {
                matches.push(rel);
            }
            Ok(())
        })?;
        matches.sort();
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

fn visit_files(
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

fn wildcard_match(pattern: &str, text: &str) -> bool {
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
