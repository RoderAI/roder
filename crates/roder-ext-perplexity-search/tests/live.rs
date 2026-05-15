use roder_ext_perplexity_search::{PerplexitySearchClient, PerplexitySearchConfig};
use roder_web_search::WebSearchRequest;

#[tokio::test]
#[ignore = "requires RODER_LIVE_WEB_SEARCH=1 and PERPLEXITY_API_KEY"]
async fn live_perplexity_smoke_returns_a_url() {
    if std::env::var("RODER_LIVE_WEB_SEARCH").ok().as_deref() != Some("1") {
        eprintln!("set RODER_LIVE_WEB_SEARCH=1 to run live Perplexity smoke test");
        return;
    }
    let Ok(api_key) = std::env::var("PERPLEXITY_API_KEY") else {
        eprintln!("set PERPLEXITY_API_KEY to run live Perplexity smoke test");
        return;
    };
    let base_url = std::env::var("PERPLEXITY_BASE_URL")
        .unwrap_or_else(|_| "https://api.perplexity.ai".to_string());
    let client =
        PerplexitySearchClient::new(PerplexitySearchConfig::new(api_key).with_base_url(base_url))
            .unwrap();

    let mut request = WebSearchRequest::new("Roder web search Rust");
    request.max_results = 1;
    let response = client.search(request).await.unwrap();

    assert!(
        response
            .results
            .iter()
            .any(|result| result.url.starts_with("http"))
    );
}
