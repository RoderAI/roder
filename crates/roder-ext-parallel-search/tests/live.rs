use roder_ext_parallel_search::{
    ParallelExtractRequest, ParallelSearchClient, ParallelSearchConfig, ParallelSearchOptions,
};
use roder_web_search::WebSearchRequest;

fn live_client() -> Option<ParallelSearchClient> {
    if std::env::var("RODER_LIVE_WEB_SEARCH").ok().as_deref() != Some("1") {
        eprintln!("set RODER_LIVE_WEB_SEARCH=1 to run live Parallel smoke tests");
        return None;
    }
    let Ok(api_key) = std::env::var("PARALLEL_API_KEY") else {
        eprintln!("set PARALLEL_API_KEY to run live Parallel smoke tests");
        return None;
    };
    let base_url = std::env::var("PARALLEL_BASE_URL")
        .unwrap_or_else(|_| "https://api.parallel.ai".to_string());
    Some(
        ParallelSearchClient::new(ParallelSearchConfig::new(api_key).with_base_url(base_url))
            .unwrap(),
    )
}

#[tokio::test]
#[ignore = "requires RODER_LIVE_WEB_SEARCH=1 and PARALLEL_API_KEY"]
async fn live_parallel_smoke_returns_a_url() {
    let Some(client) = live_client() else {
        return;
    };

    let mut request = WebSearchRequest::new("Roder web search Rust");
    request.max_results = 1;
    let response = client
        .search(request, ParallelSearchOptions::default())
        .await
        .unwrap();

    assert!(
        response
            .results
            .iter()
            .any(|result| result.url.starts_with("http"))
    );
}

#[tokio::test]
#[ignore = "requires RODER_LIVE_WEB_SEARCH=1 and PARALLEL_API_KEY"]
async fn live_parallel_extract_returns_excerpts() {
    let Some(client) = live_client() else {
        return;
    };

    let response = client
        .extract(ParallelExtractRequest {
            urls: vec!["https://docs.parallel.ai/extract/extract-quickstart".to_string()],
            objective: Some("What does the Extract API return?".to_string()),
            search_queries: Vec::new(),
            max_chars_total: Some(2000),
            session_id: None,
            full_content: false,
            max_chars_per_result: None,
        })
        .await
        .unwrap();

    assert!(response.extract_id.is_some());
    assert!(
        response
            .results
            .iter()
            .any(|result| !result.excerpts.is_empty() || result.full_content.is_some())
    );
}
