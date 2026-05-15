use std::collections::VecDeque;
use std::sync::Arc;

use roder_ext_firecrawl_search::client::FirecrawlSearchConfig;
use roder_ext_firecrawl_search::{FirecrawlSearchClient, FirecrawlSearchTool};
use roder_web_search::WebSearchRequest;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[tokio::test]
async fn search_success_normalizes_results() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "success": true,
            "data": [{
                "url": "https://example.com/roder",
                "title": "Roder",
                "description": "Roder result",
                "score": 0.95
            }]
        }),
    )])
    .await;
    let client = test_client(&server);

    let response = client.search(WebSearchRequest::new("roder")).await.unwrap();

    assert_eq!(server.requests().await.len(), 1);
    assert_eq!(response.provider, "firecrawl");
    assert_eq!(response.results[0].url, "https://example.com/roder");
    assert_eq!(response.results[0].snippet.as_deref(), Some("Roder result"));
}

#[tokio::test]
async fn include_content_sends_markdown_scrape_options() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "success": true,
            "data": [{
                "url": "https://example.com/page",
                "title": "Page",
                "markdown": "# Page"
            }]
        }),
    )])
    .await;
    let client = test_client(&server);
    let mut request = WebSearchRequest::new("page");
    request.include_content = true;

    let response = client.search(request).await.unwrap();
    let body = server.json_body(0).await;

    assert_eq!(body["scrapeOptions"]["formats"], json!(["markdown"]));
    assert_eq!(response.results[0].content.as_deref(), Some("# Page"));
}

#[tokio::test]
async fn domain_filters_are_sent_after_normalization() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "success": true,
            "data": [{ "url": "https://example.com/one" }]
        }),
    )])
    .await;
    let client = test_client(&server);
    let mut request = WebSearchRequest::new("domains");
    request.include_domains = vec!["https://www.Example.com/path".to_string()];
    request.exclude_domains = vec!["Spam.test".to_string()];

    client.search(request).await.unwrap();
    let body = server.json_body(0).await;

    assert_eq!(body["includeDomains"], json!(["example.com"]));
    assert_eq!(body["excludeDomains"], json!(["spam.test"]));
}

#[tokio::test]
async fn unauthorized_error_redacts_api_key() {
    let server = MockServer::start(vec![MockResponse::json(
        401,
        json!({ "error": { "message": "bad key secret-test-key" } }),
    )])
    .await;
    let client = test_client(&server);

    let error = client
        .search(WebSearchRequest::new("auth"))
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("[redacted]"));
    assert!(!error.contains("secret-test-key"));
}

#[tokio::test]
async fn rate_limit_error_reports_429_without_secret() {
    let server = MockServer::start(vec![
        MockResponse::json_with_headers(
            429,
            json!({ "message": "slow down" }),
            &[("Retry-After", "0")],
        ),
        MockResponse::json(429, json!({ "message": "still limited" })),
    ])
    .await;
    let client = test_client(&server);

    let error = client
        .search(WebSearchRequest::new("limits"))
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("429"));
    assert!(error.contains("still limited"));
    assert!(!error.contains("secret-test-key"));
    assert_eq!(server.requests().await.len(), 2);
}

#[tokio::test]
async fn malformed_response_is_an_error() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({ "success": true, "data": [{ "title": "Missing URL" }] }),
    )])
    .await;
    let client = test_client(&server);

    let error = client
        .search(WebSearchRequest::new("bad response"))
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("usable results"));
}

#[tokio::test]
async fn tool_result_contains_text_and_normalized_data() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "success": true,
            "data": [{ "url": "https://example.com/tool", "title": "Tool" }]
        }),
    )])
    .await;
    let tool = FirecrawlSearchTool::new(
        FirecrawlSearchConfig::new("secret-test-key").with_base_url(server.base_url()),
    )
    .unwrap();
    let call = roder_api::tools::ToolCall {
        id: "call-1".to_string(),
        name: "firecrawl_search".to_string(),
        arguments: json!({ "query": "tool" }),
        raw_arguments: r#"{"query":"tool"}"#.to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    };

    let result = roder_api::tools::ToolExecutor::execute(
        &tool,
        roder_api::tools::ToolExecutionContext {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        },
        call,
    )
    .await
    .unwrap();

    assert_eq!(result.name, "firecrawl_search");
    assert!(result.text.contains("https://example.com/tool"));
    assert_eq!(result.data["provider"], "firecrawl");
    assert!(result.data.get("raw").is_none());
}

#[tokio::test]
#[ignore = "requires RODER_LIVE_WEB_SEARCH=1 and FIRECRAWL_API_KEY"]
async fn live_firecrawl_smoke_returns_a_url() {
    if std::env::var("RODER_LIVE_WEB_SEARCH").ok().as_deref() != Some("1") {
        eprintln!("set RODER_LIVE_WEB_SEARCH=1 to run live Firecrawl smoke test");
        return;
    }
    let Ok(api_key) = std::env::var("FIRECRAWL_API_KEY") else {
        eprintln!("set FIRECRAWL_API_KEY to run live Firecrawl smoke test");
        return;
    };
    let base_url = std::env::var("FIRECRAWL_BASE_URL")
        .unwrap_or_else(|_| "https://api.firecrawl.dev".to_string());
    let client =
        FirecrawlSearchClient::new(FirecrawlSearchConfig::new(api_key).with_base_url(base_url))
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

fn test_client(server: &MockServer) -> FirecrawlSearchClient {
    FirecrawlSearchClient::new(
        FirecrawlSearchConfig::new("secret-test-key").with_base_url(server.base_url()),
    )
    .unwrap()
}

#[derive(Debug, Clone)]
struct MockResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl MockResponse {
    fn json(status: u16, body: Value) -> Self {
        Self::json_with_headers(status, body, &[])
    }

    fn json_with_headers(status: u16, body: Value, headers: &[(&str, &str)]) -> Self {
        Self {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect(),
            body: body.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    body: String,
}

#[derive(Debug)]
struct MockServer {
    address: std::net::SocketAddr,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
}

impl MockServer {
    async fn start(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_responses = responses.clone();
        let server_requests = requests.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let responses = server_responses.clone();
                let requests = server_requests.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0; 8192];
                    let mut read = 0;
                    loop {
                        let n = stream.read(&mut buffer[read..]).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        read += n;
                        if headers_complete(&buffer[..read]) {
                            break;
                        }
                        if read == buffer.len() {
                            buffer.resize(buffer.len() * 2, 0);
                        }
                    }
                    let header_end = find_header_end(&buffer[..read]).unwrap();
                    let headers = String::from_utf8_lossy(&buffer[..header_end]);
                    let content_length = content_length(&headers);
                    while read < header_end + 4 + content_length {
                        if read == buffer.len() {
                            buffer.resize(buffer.len() * 2, 0);
                        }
                        let n = stream.read(&mut buffer[read..]).await.unwrap();
                        if n == 0 {
                            break;
                        }
                        read += n;
                    }
                    let body_start = header_end + 4;
                    let body = String::from_utf8_lossy(
                        &buffer[body_start..(body_start + content_length).min(read)],
                    )
                    .to_string();
                    requests.lock().await.push(CapturedRequest { body });

                    let response = responses.lock().await.pop_front().unwrap_or_else(|| {
                        MockResponse::json(500, json!({"message":"no response"}))
                    });
                    let reason = if response.status == 200 {
                        "OK"
                    } else {
                        "Error"
                    };
                    let mut raw = format!(
                        "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
                        response.status,
                        reason,
                        response.body.len()
                    );
                    for (name, value) in response.headers {
                        raw.push_str(&format!("{name}: {value}\r\n"));
                    }
                    raw.push_str("\r\n");
                    raw.push_str(&response.body);
                    stream.write_all(raw.as_bytes()).await.unwrap();
                });
            }
        });

        Self { address, requests }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    async fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().await.clone()
    }

    async fn json_body(&self, index: usize) -> Value {
        let requests = self.requests.lock().await;
        serde_json::from_str(&requests[index].body).unwrap()
    }
}

fn headers_complete(buffer: &[u8]) -> bool {
    find_header_end(buffer).is_some()
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(headers: &str) -> usize {
    headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        .unwrap_or_default()
}
