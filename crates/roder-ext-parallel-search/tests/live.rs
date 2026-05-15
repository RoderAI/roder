use roder_ext_parallel_search::{
    ParallelSearchClient, ParallelSearchConfig, ParallelSearchOptions,
};
use roder_web_search::WebSearchRequest;

#[tokio::test]
#[ignore = "requires RODER_LIVE_WEB_SEARCH=1 and PARALLEL_API_KEY"]
async fn live_parallel_smoke_returns_a_url() {
    if std::env::var("RODER_LIVE_WEB_SEARCH").ok().as_deref() != Some("1") {
        eprintln!("set RODER_LIVE_WEB_SEARCH=1 to run live Parallel smoke test");
        return;
    }
    let Ok(api_key) = std::env::var("PARALLEL_API_KEY") else {
        eprintln!("set PARALLEL_API_KEY to run live Parallel smoke test");
        return;
    };
    let base_url = std::env::var("PARALLEL_BASE_URL")
        .unwrap_or_else(|_| "https://api.parallel.ai".to_string());
    let client =
        ParallelSearchClient::new(ParallelSearchConfig::new(api_key).with_base_url(base_url))
            .unwrap();

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
