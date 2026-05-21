use std::collections::HashSet;
use std::path::{Path, PathBuf};

use roder_api::context::{
    ContextBlock, ContextBlockKind, ContextPlan, ContextPlanner, ContextPlannerId, ContextQuery,
};
use roder_search::{DEFAULT_MAX_FILE_SIZE, SearchMode, SearchOptions, search_workspace};
use serde::Serialize;
use serde_json::json;

const MAX_CANDIDATES: usize = 5;
const MAX_FILES_SCANNED: usize = 2_000;
const MAX_CONTENT_BYTES: u64 = 8 * 1024;

#[derive(Debug, Clone)]
pub struct EntrypointContextPlanner {
    workspace: PathBuf,
}

impl EntrypointContextPlanner {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }
}

#[async_trait::async_trait]
impl ContextPlanner for EntrypointContextPlanner {
    fn id(&self) -> ContextPlannerId {
        "entrypoint-context-planner".to_string()
    }

    async fn plan(
        &self,
        query: &ContextQuery,
        mut provider_blocks: Vec<ContextBlock>,
    ) -> anyhow::Result<ContextPlan> {
        let candidates = discover_entrypoints(&self.workspace, &query.prompt)?;
        if !candidates.is_empty() {
            provider_blocks.push(render_entrypoint_block(&candidates));
        }
        provider_blocks.sort_by_key(|block| std::cmp::Reverse(block.priority));
        Ok(ContextPlan {
            blocks: provider_blocks,
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct EntrypointCandidate {
    path: String,
    score: i32,
    reasons: Vec<String>,
}

fn discover_entrypoints(root: &Path, prompt: &str) -> anyhow::Result<Vec<EntrypointCandidate>> {
    let tokens = prompt_tokens(prompt);
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let changed = changed_paths(root);
    let search_hits = fresh_search_hits(root, &tokens);
    let mut candidates = Vec::new();
    let mut scanned = 0usize;
    visit_files(root, &mut |path| {
        if scanned >= MAX_FILES_SCANNED {
            return Ok(());
        }
        scanned += 1;
        if let Some(candidate) = score_file(root, path, &tokens, &changed, &search_hits)? {
            candidates.push(candidate);
        }
        Ok(())
    })?;
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    candidates.truncate(MAX_CANDIDATES);
    Ok(candidates)
}

fn score_file(
    root: &Path,
    path: &Path,
    tokens: &[String],
    changed: &HashSet<String>,
    search_hits: &HashSet<String>,
) -> anyhow::Result<Option<EntrypointCandidate>> {
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let rel_lower = rel.to_ascii_lowercase();
    let file_type_score = extension_score(path);
    let mut score = 0;
    let mut reasons = Vec::new();

    for token in tokens {
        if rel_lower.contains(token) {
            score += 8;
            reasons.push(format!("path matches `{token}`"));
        }
    }

    if changed.contains(&rel) {
        score += 10;
        reasons.push("recent git change".to_string());
    }

    if search_hits.contains(&rel) {
        score += 6;
        reasons.push("fresh search hit".to_string());
    }

    if likely_entrypoint_name(path) {
        score += 4;
        reasons.push("entrypoint-like filename".to_string());
    }

    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.len() <= MAX_CONTENT_BYTES {
            if let Ok(text) = std::fs::read_to_string(path) {
                let text = text.to_ascii_lowercase();
                for token in tokens {
                    if text.contains(token) {
                        score += 3;
                        reasons.push(format!("bounded content matches `{token}`"));
                    }
                }
            }
        }
    }

    Ok((score > 0).then_some(EntrypointCandidate {
        path: rel,
        score: score + file_type_score,
        reasons,
    }))
}

fn render_entrypoint_block(candidates: &[EntrypointCandidate]) -> ContextBlock {
    let mut text = String::from("Likely entry points:");
    for (index, candidate) in candidates.iter().enumerate() {
        let reason = candidate
            .reasons
            .first()
            .map(String::as_str)
            .unwrap_or("workspace evidence");
        text.push_str(&format!("\n{}. {} - {}", index + 1, candidate.path, reason));
    }

    ContextBlock {
        id: "entrypoint-context-planner".to_string(),
        kind: ContextBlockKind::EntrypointHint,
        text,
        priority: 90,
        token_estimate: None,
        metadata: json!({
            "planner": "entrypoint-context-planner",
            "candidate_count": candidates.len(),
            "candidates": candidates,
            "source": "fresh_filesystem_heuristics",
        }),
    }
}

fn visit_files(
    root: &Path,
    visitor: &mut dyn FnMut(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if root.is_file() {
        return visitor(root);
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type()?.is_dir() {
            if matches!(name.as_ref(), ".git" | "target" | "node_modules" | ".roder") {
                continue;
            }
            visit_files(&path, visitor)?;
        } else {
            visitor(&path)?;
        }
    }
    Ok(())
}

fn prompt_tokens(prompt: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    prompt
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| token.len() >= 3 && !STOP_WORDS.contains(&token.as_str()))
        .filter(|token| seen.insert(token.clone()))
        .collect()
}

fn extension_score(path: &Path) -> i32 {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs" | "toml" | "md") => 2,
        Some("ts" | "tsx" | "js" | "jsx" | "py") => 1,
        _ => 0,
    }
}

fn likely_entrypoint_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        name,
        "lib.rs" | "main.rs" | "mod.rs" | "runtime.rs" | "server.rs" | "index.ts" | "index.tsx"
    )
}

fn changed_paths(root: &Path) -> HashSet<String> {
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("status")
        .arg("--short")
        .output()
    else {
        return HashSet::new();
    };
    if !output.status.success() {
        return HashSet::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.get(3..))
        .map(|path| path.trim().replace('\\', "/"))
        .collect()
}

fn fresh_search_hits(root: &Path, tokens: &[String]) -> HashSet<String> {
    let mut hits = HashSet::new();
    for token in tokens.iter().take(6) {
        let mut options = SearchOptions::new(token.clone())
            .with_mode(SearchMode::Scan)
            .case_sensitive(false);
        options.max_file_size = DEFAULT_MAX_FILE_SIZE.min(MAX_CONTENT_BYTES);
        let Ok(results) = search_workspace(root, &options) else {
            continue;
        };
        for hit in results.matches.iter().take(100) {
            hits.insert(hit.path.to_string_lossy().replace('\\', "/"));
        }
    }
    hits
}

const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "where", "when", "what", "why",
    "how", "need", "needs", "find", "file", "files", "code", "task", "work",
];

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::context::ContextQuery;

    #[tokio::test]
    async fn entrypoint_planner_puts_relevant_file_in_top_five() {
        let root = test_workspace("entrypoint-top-five");
        write(
            &root,
            "crates/roder-core/src/runtime.rs",
            "fn route_tools() {}\n",
        );
        write(
            &root,
            "crates/roder-tools/src/files.rs",
            "fn read_file() {}\n",
        );
        write(&root, "README.md", "Roder docs\n");
        let planner = EntrypointContextPlanner::new(root.clone());

        let plan = planner
            .plan(&query("debug runtime tool routing"), Vec::new())
            .await
            .unwrap();

        let block = plan.blocks.first().unwrap();
        assert!(block.text.contains("crates/roder-core/src/runtime.rs"));
        assert_eq!(block.kind, ContextBlockKind::EntrypointHint);
        assert!(block.text.lines().nth(1).unwrap().contains("runtime.rs"));
        assert!(block.metadata["candidate_count"].as_u64().unwrap() <= MAX_CANDIDATES as u64);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn entrypoint_planner_keeps_output_bounded_for_large_files() {
        let root = test_workspace("entrypoint-bounded");
        write(&root, "src/runtime.rs", &"runtime ".repeat(4_000));
        let planner = EntrypointContextPlanner::new(root.clone());

        let plan = planner
            .plan(&query("runtime entrypoint"), Vec::new())
            .await
            .unwrap();
        let block = plan.blocks.first().unwrap();

        assert!(block.text.len() < 1_000);
        assert!(!block.text.contains(&"runtime ".repeat(100)));

        let _ = std::fs::remove_dir_all(root);
    }

    fn query(prompt: &str) -> ContextQuery {
        ContextQuery {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            prompt: prompt.to_string(),
            token_budget: None,
        }
    }

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn test_workspace(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("roder-context-{name}-{stamp}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
