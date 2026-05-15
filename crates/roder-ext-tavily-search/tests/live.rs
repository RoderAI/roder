use roder_ext_tavily_search::{TavilySearchClient, TavilySearchConfig};
use roder_web_search::WebSearchRequest;

#[tokio::test]
#[ignore = "requires RODER_LIVE_WEB_SEARCH=1 and TAVILY_API_KEY"]
async fn live_tavily_smoke_returns_a_url() {
    if std::env::var("RODER_LIVE_WEB_SEARCH").ok().as_deref() != Some("1") {
        eprintln!("set RODER_LIVE_WEB_SEARCH=1 to run live Tavily smoke test");
        return;
    }
    let Ok(api_key) = std::env::var("TAVILY_API_KEY") else {
        eprintln!("set TAVILY_API_KEY to run live Tavily smoke test");
        return;
    };
    let base_url =
        std::env::var("TAVILY_BASE_URL").unwrap_or_else(|_| "https://api.tavily.com".to_string());
    let mut config = TavilySearchConfig::new(api_key).with_base_url(base_url);
    if let Ok(project_id) = std::env::var("TAVILY_PROJECT") {
        config = config.with_project_id(project_id);
    }
    let client = TavilySearchClient::new(config).unwrap();

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
