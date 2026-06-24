use roder_ext_synthetic_search::{SyntheticSearchClient, SyntheticSearchConfig};
use roder_web_search::WebSearchRequest;

#[tokio::test]
#[ignore = "requires RODER_LIVE_WEB_SEARCH=1 and SYNTHETIC_API_KEY"]
async fn live_synthetic_smoke_returns_a_url() {
    if std::env::var("RODER_LIVE_WEB_SEARCH").ok().as_deref() != Some("1") {
        eprintln!("set RODER_LIVE_WEB_SEARCH=1 to run live Synthetic smoke test");
        return;
    }
    let Ok(api_key) = std::env::var("SYNTHETIC_API_KEY") else {
        eprintln!("set SYNTHETIC_API_KEY to run live Synthetic smoke test");
        return;
    };
    let base_url = std::env::var("SYNTHETIC_BASE_URL")
        .unwrap_or_else(|_| "https://api.synthetic.new".to_string());
    let client =
        SyntheticSearchClient::new(SyntheticSearchConfig::new(api_key).with_base_url(base_url))
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
