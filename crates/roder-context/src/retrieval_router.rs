use roder_api::context::{
    ContextBlock, ContextBlockKind, ContextPlan, ContextPlanner, ContextPlannerId, ContextQuery,
};
use roder_api::retrieval::{
    RetrievalAvoidance, RetrievalConfidence, RetrievalIntent, RetrievalMode,
    RetrievalRecommendation, RetrievalRoutePlan,
};
use serde::Serialize;
use serde_json::json;
use time::OffsetDateTime;

#[derive(Debug, Clone, Default)]
pub struct RetrievalRouterPlanner;

#[async_trait::async_trait]
impl ContextPlanner for RetrievalRouterPlanner {
    fn id(&self) -> ContextPlannerId {
        "retrieval-router".to_string()
    }

    async fn plan(
        &self,
        query: &ContextQuery,
        mut provider_blocks: Vec<ContextBlock>,
    ) -> anyhow::Result<ContextPlan> {
        let plan = route_retrieval(query, &provider_blocks);
        if !plan.recommended.is_empty() || !plan.avoid.is_empty() {
            provider_blocks.push(render_retrieval_block(&plan));
        }
        provider_blocks.sort_by_key(|block| std::cmp::Reverse(block.priority));
        Ok(ContextPlan {
            blocks: provider_blocks,
        })
    }
}

pub fn route_retrieval(
    query: &ContextQuery,
    provider_blocks: &[ContextBlock],
) -> RetrievalRoutePlan {
    let prompt = query.prompt.to_ascii_lowercase();
    let mut recommended = Vec::new();
    let mut avoid = Vec::new();
    let intent = classify_intent(&prompt);
    let semantic_ready = provider_blocks.iter().any(|block| {
        block
            .metadata
            .get("source")
            .and_then(serde_json::Value::as_str)
            == Some("indexed_semantic_code_search")
    });

    if looks_like_command_failure(&prompt) {
        recommended.push(recommend(
            RetrievalMode::Artifact,
            "grep_artifact",
            extract_query(&query.prompt),
            "command or terminal output failure should start with saved artifact search",
            RetrievalConfidence::High,
        ));
    }
    if looks_like_capability_lookup(&prompt) {
        if let Some(item_id) = matching_promoted_capability(provider_blocks, &prompt) {
            let mut rec = recommend(
                RetrievalMode::Promotion,
                "discovery.read",
                extract_query(&query.prompt),
                "matching capability is already promoted or warm-cached for this session",
                RetrievalConfidence::High,
            );
            rec.item_id = Some(item_id);
            recommended.push(rec);
        }
        recommended.push(recommend(
            RetrievalMode::Discovery,
            "discovery.search",
            extract_query(&query.prompt),
            "tool, MCP, skill, command, or plugin capability lookup",
            RetrievalConfidence::High,
        ));
    }
    if looks_like_capability_execution(&prompt) {
        recommended.push(recommend(
            RetrievalMode::Promotion,
            "discovery.read",
            extract_query(&query.prompt),
            "full schema or instructions are needed before capability use",
            RetrievalConfidence::High,
        ));
    }
    if looks_like_file_name(&query.prompt) {
        recommended.push(recommend(
            RetrievalMode::FileName,
            "glob",
            extract_query(&query.prompt),
            "path or filename-shaped prompt",
            RetrievalConfidence::High,
        ));
    }
    if looks_like_exact_search(&query.prompt) {
        recommended.push(recommend(
            RetrievalMode::ExactText,
            "grep",
            extract_query(&query.prompt),
            "exact symbol, path, regex, or error string",
            RetrievalConfidence::High,
        ));
    }
    if matches!(intent, RetrievalIntent::BroadConcept) {
        if semantic_ready {
            recommended.push(recommend(
                RetrievalMode::SemanticCode,
                "code_index.search",
                extract_query(&query.prompt),
                "conceptual code search with ready semantic index",
                RetrievalConfidence::Medium,
            ));
        } else {
            recommended.push(recommend(
                RetrievalMode::ExactText,
                "grep",
                extract_query(&query.prompt),
                "semantic index not observed; start with exact local search fallback",
                RetrievalConfidence::Medium,
            ));
        }
    }
    if prompt.contains("history") || prompt.contains("previous turn") || prompt.contains("resume") {
        recommended.push(recommend(
            RetrievalMode::History,
            "history.search",
            extract_query(&query.prompt),
            "prior conversation or session recovery",
            RetrievalConfidence::Medium,
        ));
    }
    if prompt.contains("code") || prompt.contains("repo") || prompt.contains("workspace") {
        avoid.push(RetrievalAvoidance {
            mode: RetrievalMode::Web,
            reason: "local workspace retrieval should be tried before web search".to_string(),
        });
    }

    dedupe_recommendations(&mut recommended);
    RetrievalRoutePlan {
        route_id: format!("route:{}:{}", query.thread_id, query.turn_id),
        thread_id: query.thread_id.clone(),
        turn_id: query.turn_id.clone(),
        intent,
        recommended,
        avoid,
        timestamp: OffsetDateTime::now_utc(),
    }
}

fn render_retrieval_block(plan: &RetrievalRoutePlan) -> ContextBlock {
    let mut text = format!("Retrieval route intent: {:?}", plan.intent);
    for (index, rec) in plan.recommended.iter().take(5).enumerate() {
        text.push_str(&format!(
            "\n{}. {:?} via `{}` query `{}` - {}",
            index + 1,
            rec.mode,
            rec.tool,
            truncate(&rec.query, 80),
            rec.reason
        ));
    }
    if !plan.avoid.is_empty() {
        let avoid = plan
            .avoid
            .iter()
            .map(|avoid| format!("{:?}: {}", avoid.mode, avoid.reason))
            .collect::<Vec<_>>()
            .join("; ");
        text.push_str(&format!("\nAvoid: {avoid}"));
    }

    ContextBlock {
        id: "retrieval-router".to_string(),
        kind: ContextBlockKind::RetrievalHint,
        text,
        priority: 88,
        token_estimate: None,
        metadata: json!({
            "planner": "retrieval-router",
            "route_id": plan.route_id,
            "intent": format!("{:?}", plan.intent),
            "recommended": serializable(&plan.recommended),
            "avoid": serializable(&plan.avoid),
        }),
    }
}

fn classify_intent(prompt: &str) -> RetrievalIntent {
    if prompt.contains("tool")
        || prompt.contains("mcp")
        || prompt.contains("skill")
        || prompt.contains("plugin")
    {
        return RetrievalIntent::InspectTool;
    }
    if looks_like_command_failure(prompt) {
        return RetrievalIntent::DebugFailure;
    }
    if prompt.contains("usage") || prompt.contains("call sites") || prompt.contains("where used") {
        return RetrievalIntent::TraceUsage;
    }
    if prompt.contains("history") || prompt.contains("previous turn") || prompt.contains("resume") {
        return RetrievalIntent::RecoverHistory;
    }
    if prompt.contains("file") || prompt.contains("path") || prompt.contains("filename") {
        return RetrievalIntent::FileLookup;
    }
    if looks_like_exact_search(prompt) {
        return RetrievalIntent::FindDefinition;
    }
    RetrievalIntent::BroadConcept
}

fn recommend(
    mode: RetrievalMode,
    tool: &str,
    query: String,
    reason: &str,
    confidence: RetrievalConfidence,
) -> RetrievalRecommendation {
    RetrievalRecommendation {
        mode,
        tool: tool.to_string(),
        query,
        reason: reason.to_string(),
        confidence,
        item_id: None,
    }
}

fn dedupe_recommendations(recommended: &mut Vec<RetrievalRecommendation>) {
    let mut seen = std::collections::BTreeSet::new();
    recommended.retain(|rec| seen.insert((rec.mode.clone(), rec.tool.clone())));
    recommended.truncate(5);
}

fn looks_like_capability_lookup(prompt: &str) -> bool {
    prompt.contains("tool")
        || prompt.contains("mcp")
        || prompt.contains("skill")
        || prompt.contains("command")
        || prompt.contains("plugin")
}

fn looks_like_capability_execution(prompt: &str) -> bool {
    looks_like_capability_lookup(prompt)
        && (prompt.contains("run")
            || prompt.contains("use")
            || prompt.contains("execute")
            || prompt.contains("call")
            || prompt.contains("invoke"))
}

fn looks_like_command_failure(prompt: &str) -> bool {
    prompt.contains("stderr")
        || prompt.contains("stdout")
        || prompt.contains("exit code")
        || prompt.contains("terminal")
        || prompt.contains("command failed")
        || prompt.contains("panic")
        || prompt.contains("stack trace")
}

fn looks_like_file_name(prompt: &str) -> bool {
    prompt.contains('/')
        || prompt.contains(".rs")
        || prompt.contains(".ts")
        || prompt.contains(".tsx")
        || prompt.contains(".json")
        || prompt.contains(".toml")
        || prompt.contains(".md")
}

fn looks_like_exact_search(prompt: &str) -> bool {
    prompt.contains("::")
        || prompt.contains("->")
        || prompt.contains("fn ")
        || prompt.contains("struct ")
        || prompt.contains("enum ")
        || prompt.split_whitespace().any(|token| {
            token.len() >= 4
                && token
                    .chars()
                    .any(|ch| ch == '_' || ch.is_ascii_uppercase() || ch.is_ascii_digit())
        })
}

fn matching_promoted_capability(blocks: &[ContextBlock], prompt: &str) -> Option<String> {
    let prompt_tokens = prompt
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .map(str::to_ascii_lowercase)
        .filter(|token| token.len() >= 3)
        .collect::<Vec<_>>();
    blocks.iter().find_map(|block| {
        let source = block
            .metadata
            .get("source")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !matches!(
            source,
            "promoted_capabilities" | "discovery_promotions" | "warm_cached_capabilities"
        ) {
            return None;
        }
        let item_id = block
            .metadata
            .get("item_id")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                block
                    .metadata
                    .get("itemId")
                    .and_then(serde_json::Value::as_str)
            })?;
        let haystack = format!("{} {}", item_id, block.text).to_ascii_lowercase();
        prompt_tokens
            .iter()
            .any(|token| haystack.contains(token))
            .then(|| item_id.to_string())
    })
}

fn extract_query(prompt: &str) -> String {
    truncate(prompt.trim(), 120).to_string()
}

fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn serializable<T: Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_else(|_| serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retrieval_router_routes_exact_symbols_to_grep() {
        let planner = RetrievalRouterPlanner;
        let plan = planner
            .plan(&query("Find ToolExecutionContext in the repo"), Vec::new())
            .await
            .unwrap();

        let block = plan
            .blocks
            .iter()
            .find(|block| block.kind == ContextBlockKind::RetrievalHint)
            .unwrap();
        assert!(block.text.contains("ExactText"));
        assert!(block.text.contains("`grep`"));
    }

    #[tokio::test]
    async fn retrieval_router_routes_concepts_to_semantic_when_index_ready() {
        let planner = RetrievalRouterPlanner;
        let semantic_block = ContextBlock {
            id: "code-index".to_string(),
            kind: ContextBlockKind::RetrievedDocument,
            text: "Indexed context".to_string(),
            priority: 10,
            token_estimate: None,
            metadata: json!({ "source": "indexed_semantic_code_search" }),
        };

        let plan = planner
            .plan(
                &query("How does the policy gate choose approvals?"),
                vec![semantic_block],
            )
            .await
            .unwrap();

        let block = plan.blocks.first().unwrap();
        assert_eq!(block.kind, ContextBlockKind::RetrievalHint);
        assert!(block.text.contains("SemanticCode"));
        assert!(block.text.contains("code_index.search"));
    }

    #[tokio::test]
    async fn retrieval_router_routes_capability_execution_to_discovery_and_promotion() {
        let planner = RetrievalRouterPlanner;
        let plan = planner
            .plan(
                &query("Use the GitHub MCP issue search tool to find blockers"),
                Vec::new(),
            )
            .await
            .unwrap();
        let block = plan.blocks.first().unwrap();

        assert!(block.text.contains("Discovery"));
        assert!(block.text.contains("Promotion"));
        assert!(block.text.contains("discovery.search"));
        assert!(block.text.contains("discovery.read"));
    }

    #[tokio::test]
    async fn retrieval_router_prefers_promoted_capability_state() {
        let planner = RetrievalRouterPlanner;
        let promoted = ContextBlock {
            id: "promoted-github".to_string(),
            kind: ContextBlockKind::ToolAvailability,
            text: "GitHub issue search is promoted".to_string(),
            priority: 20,
            token_estimate: None,
            metadata: json!({
                "source": "promoted_capabilities",
                "item_id": "mcp:github/issues.search",
            }),
        };

        let plan = planner
            .plan(
                &query("Use the GitHub MCP issue search tool"),
                vec![promoted],
            )
            .await
            .unwrap();
        let block = plan.blocks.first().unwrap();

        assert!(block.text.contains("already promoted or warm-cached"));
        assert_eq!(
            block.metadata["recommended"][0]["itemId"],
            "mcp:github/issues.search"
        );
    }

    #[tokio::test]
    async fn retrieval_router_routes_command_failures_to_artifacts() {
        let planner = RetrievalRouterPlanner;
        let plan = planner
            .plan(
                &query("A terminal command failed with stderr; inspect the log"),
                Vec::new(),
            )
            .await
            .unwrap();
        let block = plan.blocks.first().unwrap();

        assert!(block.text.contains("Artifact"));
        assert!(block.text.contains("grep_artifact"));
    }

    fn query(prompt: &str) -> ContextQuery {
        ContextQuery {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            prompt: prompt.to_string(),
            token_budget: None,
        }
    }
}
