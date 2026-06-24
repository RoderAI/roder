use std::collections::VecDeque;
use std::sync::Arc;

use roder_ext_synthetic_search::client::SyntheticSearchConfig;
use roder_ext_synthetic_search::{SyntheticSearchClient, SyntheticSearchTool};
use roder_web_search::WebSearchRequest;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[tokio::test]
async fn basic_search_posts_query_and_normalizes_results() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "request_id": "req_basic",
            "results": [{
                "url": "https://example.com/roder",
                "title": "Roder",
                "text": "Roder result",
                "published": "2026-06-22",
                "score": 0.95
            }]
        }),
    )])
    .await;
    let client = test_client(&server);

    let response = client.search(WebSearchRequest::new("roder")).await.unwrap();
    let body = server.json_body(0).await;
    let headers = &server.requests().await[0].headers;

    assert_eq!(server.requests().await.len(), 1);
    assert_eq!(body["query"], "roder");
    assert_eq!(body["max_results"], 5);
    assert_eq!(body["max_tokens_per_page"], 1_000);
    assert!(headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("authorization") && value == "Bearer secret-test-key"
    }));
    assert!(headers
        .iter()
        .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
            && value.starts_with("application/json")));
    assert_eq!(response.provider, "synthetic");
    assert_eq!(response.results[0].url, "https://example.com/roder");
    assert_eq!(response.results[0].title.as_deref(), Some("Roder"));
    assert_eq!(response.results[0].snippet.as_deref(), Some("Roder result"));
    assert_eq!(
        response.results[0].published_at.as_deref(),
        Some("2026-06-22")
    );
    assert_eq!(response.results[0].score, Some(0.95));
}

#[tokio::test]
async fn include_and_exclude_domains_are_forwarded() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "results": [{ "url": "https://example.com/page" }]
        }),
    )])
    .await;
    let client = test_client(&server);

    let mut request = WebSearchRequest::new("advanced");
    request.include_domains = vec!["example.com".to_string()];
    request.exclude_domains = vec!["spam.test".to_string()];
    client.search(request).await.unwrap();
    let body = server.json_body(0).await;

    assert_eq!(body["include_domains"], json!(["example.com"]));
    assert_eq!(body["exclude_domains"], json!(["spam.test"]));
    assert!(body.get("country").is_none());
    assert!(body.get("freshness").is_none());
}

#[tokio::test]
async fn max_text_length_is_configurable() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "results": [{ "url": "https://example.com/page" }]
        }),
    )])
    .await;
    let client = SyntheticSearchClient::new(
        SyntheticSearchConfig::new("secret-test-key")
            .with_base_url(server.base_url())
            .with_max_text_length(5_000),
    )
    .unwrap();

    client.search(WebSearchRequest::new("size")).await.unwrap();
    let body = server.json_body(0).await;

    assert_eq!(body["max_tokens_per_page"], 5_000);
}

#[tokio::test]
async fn usage_and_request_id_are_available_in_tool_result_data() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "request_id": "req_usage",
            "results": [{ "url": "https://example.com/tool", "title": "Tool" }],
            "usage": { "tokens": 42 }
        }),
    )])
    .await;
    let tool = SyntheticSearchTool::new(
        SyntheticSearchConfig::new("secret-test-key").with_base_url(server.base_url()),
    )
    .unwrap();
    let call = roder_api::tools::ToolCall {
        id: "call-1".to_string(),
        name: "synthetic_search".to_string(),
        arguments: json!({ "query": "tool" }),
        raw_arguments: r#"{"query":"tool"}"#.to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    };

    let result = roder_api::tools::ToolExecutor::execute(
        &tool,
        roder_api::tools::ToolExecutionContext::new(
            "thread-1",
            "turn-1",
            roder_api::policy_mode::PolicyMode::Default,
        ),
        call,
    )
    .await
    .unwrap();

    assert_eq!(result.name, "synthetic_search");
    assert!(result.text.contains("https://example.com/tool"));
    assert_eq!(result.data["provider"], "synthetic");
    assert_eq!(result.data["provider_request_id"], "req_usage");
    assert_eq!(result.data["usage"]["provider_metadata"]["tokens"], 42);
    assert!(result.data.get("raw").is_none());
}

#[tokio::test]
async fn unauthorized_error_redacts_api_key() {
    let server = MockServer::start(vec![MockResponse::json(
        401,
        json!({ "error": { "message": "bad secret-test-key" } }),
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
            json!({ "message": "slow down secret-test-key" }),
            &[("Retry-After", "0")],
        ),
        MockResponse::json(429, json!({ "message": "still limited secret-test-key" })),
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
async fn validation_errors_do_not_send_requests() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({ "results": [{ "url": "https://example.com/unused" }] }),
    )])
    .await;
    let client = test_client(&server);
    let mut request = WebSearchRequest::new(" ");
    request.max_results = 21;

    let error = client.search(request).await.unwrap_err().to_string();

    assert!(error.contains("query is required"));
    assert!(server.requests().await.is_empty());
}

#[tokio::test]
async fn empty_results_return_an_error() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({ "results": [] }),
    )])
    .await;
    let client = test_client(&server);

    let error = client
        .search(WebSearchRequest::new("nothing"))
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("did not contain any usable results"));
}

#[tokio::test]
async fn rejects_empty_api_key() {
    let error = SyntheticSearchClient::new(SyntheticSearchConfig::new(""))
        .err()
        .map(|err| err.to_string())
        .unwrap_or_default();

    assert!(error.contains("Synthetic API key is required"));
}

fn test_client(server: &MockServer) -> SyntheticSearchClient {
    SyntheticSearchClient::new(
        SyntheticSearchConfig::new("secret-test-key").with_base_url(server.base_url()),
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
    headers: Vec<(String, String)>,
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
                    let headers_text = String::from_utf8_lossy(&buffer[..header_end]).to_string();
                    let content_length = content_length(&headers_text);
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
                    requests.lock().await.push(CapturedRequest {
                        headers: parse_headers(&headers_text),
                        body,
                    });

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

fn parse_headers(headers: &str) -> Vec<(String, String)> {
    headers
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.to_string(), value.trim().to_string()))
        })
        .collect()
}
