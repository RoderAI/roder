use serde_json::json;

use crate::types::{WebSearchResponse, WebSearchResult, WebSearchUsage};

pub fn sample_response() -> WebSearchResponse {
    WebSearchResponse {
        provider: "parallel".to_string(),
        query: "roder web search".to_string(),
        results: vec![WebSearchResult {
            title: Some("Roder web search".to_string()),
            url: "https://example.com/roder".to_string(),
            snippet: Some("A compact result excerpt.".to_string()),
            content: Some("Longer page content.".to_string()),
            published_at: Some("2026-05-15".to_string()),
            score: Some(0.98),
            source: Some("example".to_string()),
            metadata: json!({ "rank": 1 }),
        }],
        answer: Some("Roder can normalize provider search results.".to_string()),
        usage: Some(WebSearchUsage {
            requests: Some(1),
            input_tokens: Some(12),
            output_tokens: Some(34),
            cost_usd: Some(0.001),
            provider_metadata: json!({ "plan": "test" }),
        }),
        warnings: Vec::new(),
        raw: json!({ "request_id": "req_123", "secret": "not emitted by default" }),
    }
}
