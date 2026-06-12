use std::path::Path;
use std::sync::Arc;

use ignore::WalkBuilder;
use regex::RegexBuilder;
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use roder_search::{DEFAULT_MAX_FILE_SIZE, SearchEngine, SearchMetadata, SearchMode, SearchOptions};
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
            description: "Search text files for a regular expression (default) or literal query with paginated output. Relative paths resolve from the workspace root; absolute paths are searched directly."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Regular expression to search for (set regex:false for a literal string)." },
                    "path": {
                        "type": "string",
                        "default": ".",
                        "description": "Directory or file to search. Relative paths resolve from the workspace root; an absolute path is searched directly."
                    },
                    "regex": {
                        "type": "boolean",
                        "default": true,
                        "description": "Treat query as a regular expression (default). Set false to match the query as a literal string."
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
        let regex = args.regex.unwrap_or(true);
        let case_sensitive = args.case_sensitive.unwrap_or(true);
        let word_boundary = args.word_boundary.unwrap_or(false);
        let mode = args.mode.clone().unwrap_or_else(|| "auto".to_string());
        let response_format = args.response_format.unwrap_or_default();
        if regex
            && let Err(err) = RegexBuilder::new(&query)
                .case_insensitive(!case_sensitive)
                .build()
        {
            anyhow::bail!(
                "invalid regex {query:?}: {err}\nTo search for this text literally, set \"regex\": false."
            );
        }
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
        // An empty success is indistinguishable from a broken tool; always
        // tell the model what was searched and how, plus likely remedies.
        if page.total == 0 {
            text = no_match_message(&query, regex, case_sensitive, &start, &metadata);
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
                "Find files under the workspace root matching a glob pattern with paginated output. Patterns are workspace-relative and support *, ?, [..], {a,b} and **."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Workspace-relative glob pattern, e.g. src/**/*.{ts,tsx}." },
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
        let outcome = backend.glob(&args.pattern).await?;
        let matches = outcome.matches;
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
        if page.total == 0 {
            text = format!(
                "No files matched pattern {:?} ({} files considered under the workspace root). Patterns are workspace-relative and support *, ?, [..], {{a,b}} and **.",
                args.pattern, outcome.files_considered
            );
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
        data["files_considered"] = json!(outcome.files_considered);
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
            regex: self.regex.unwrap_or(true),
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

pub(crate) struct GlobOutcome {
    pub(crate) matches: Vec<String>,
    pub(crate) files_considered: usize,
}

fn looks_like_regex(query: &str) -> bool {
    query.chars().any(|ch| {
        matches!(
            ch,
            '|' | '(' | ')' | '[' | ']' | '{' | '}' | '*' | '+' | '?' | '^' | '$' | '\\'
        )
    })
}

fn no_match_message(
    query: &str,
    regex: bool,
    case_sensitive: bool,
    start: &str,
    metadata: &SearchMetadata,
) -> String {
    let mode = if regex { "regex" } else { "literal string" };
    let case = if case_sensitive {
        "case-sensitive"
    } else {
        "case-insensitive"
    };
    let scope = if start.is_empty() || start == "." {
        "the workspace root"
    } else {
        start
    };
    let mut text = format!(
        "No matches for {query:?} ({mode}, {case}) under {scope}. engine={}, files_scanned={}, candidate_files={}.",
        metadata.engine.as_str(),
        metadata.verified_files,
        metadata.candidate_files,
    );
    if !regex && looks_like_regex(query) {
        text.push_str(
            "\nHint: the query contains regex syntax but was matched as a literal string; retry with \"regex\": true.",
        );
    }
    // Only the local scan engine reports a true files-walked count; the
    // runner backend reports matches only, so zero there is inconclusive.
    if metadata.candidate_files == 0
        && metadata.verified_files == 0
        && matches!(metadata.engine, SearchEngine::Scan)
    {
        text.push_str(
            "\nHint: no searchable text files were found under this path; it may be empty or contain only binary or oversized files.",
        );
    }
    text
}

/// Resolve a glob pattern to a workspace-relative form. Absolute and
/// home-relative patterns are accepted when they point inside the workspace
/// root and rejected with a clear error otherwise, instead of silently
/// matching nothing.
pub(crate) fn prepare_glob_pattern(root: &Path, pattern: &str) -> anyhow::Result<String> {
    let trimmed = pattern.trim();
    let expanded = if trimmed == "~" || trimmed.starts_with("~/") {
        crate::workspace::expand_home(trimmed)?
            .to_string_lossy()
            .into_owned()
    } else {
        trimmed.to_string()
    };
    let normalized = expanded.replace('\\', "/");
    if !Path::new(&normalized).is_absolute() {
        return Ok(normalize_relative_pattern(&normalized));
    }
    // Split off the literal directory prefix (everything before the first
    // wildcard component) so symlinked spellings of the workspace root, like
    // /var vs /private/var on macOS, still resolve inside the workspace.
    let mut literal_parts = Vec::new();
    let mut wildcard_parts = Vec::new();
    for part in normalized.split('/') {
        if !wildcard_parts.is_empty()
            || part
                .chars()
                .any(|ch| matches!(ch, '*' | '?' | '[' | '{'))
        {
            wildcard_parts.push(part);
        } else {
            literal_parts.push(part);
        }
    }
    let literal_prefix = literal_parts.join("/");
    let prefix_path = Path::new(&literal_prefix);
    let resolved_prefix = prefix_path
        .canonicalize()
        .unwrap_or_else(|_| prefix_path.to_path_buf());
    let Ok(inside) = resolved_prefix.strip_prefix(root) else {
        anyhow::bail!(
            "glob pattern {pattern:?} points outside the workspace root {}; glob only searches the workspace — pass a workspace-relative pattern, or use list_files/grep with an absolute path",
            root.display()
        );
    };
    let mut relative = inside.to_string_lossy().replace('\\', "/");
    if !wildcard_parts.is_empty() {
        if !relative.is_empty() {
            relative.push('/');
        }
        relative.push_str(&wildcard_parts.join("/"));
    }
    Ok(normalize_relative_pattern(&relative))
}

pub(crate) fn compile_glob(pattern: &str) -> anyhow::Result<globset::GlobMatcher> {
    globset::GlobBuilder::new(pattern)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|err| anyhow::anyhow!("invalid glob pattern {pattern:?}: {err}"))
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
