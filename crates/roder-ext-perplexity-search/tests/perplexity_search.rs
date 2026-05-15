use std::collections::VecDeque;
use std::sync::Arc;

use roder_ext_perplexity_search::client::PerplexitySearchConfig;
use roder_ext_perplexity_search::{PerplexitySearchClient, PerplexitySearchTool};
use roder_web_search::{Freshness, WebSearchRequest};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[tokio::test]
async fn raw_search_sends_bearer_auth_and_normalizes_results() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "request_id": "req_raw",
            "results": [{
                "url": "https://example.com/roder",
                "title": "Roder",
                "snippet": "Roder result",
                "date": "2026-05-14",
                "last_updated": "2026-05-15",
                "score": 0.95,
                "citations": ["https://example.com/source"]
            }],
            "usage": { "searches": 1, "input_tokens": 12, "output_tokens": 34 }
        }),
    )])
    .await;
    let client = test_client(&server);

    let response = client.search(WebSearchRequest::new("roder")).await.unwrap();
    let body = server.json_body(0).await;
    let requests = server.requests().await;

    assert_eq!(requests.len(), 1);
    assert!(requests[0].headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("authorization") && value == "Bearer secret-test-key"
    }));
    assert_eq!(body["query"], "roder");
    assert_eq!(body["max_results"], 5);
    assert_eq!(response.provider, "perplexity");
    assert_eq!(response.results[0].url, "https://example.com/roder");
    assert_eq!(response.results[0].snippet.as_deref(), Some("Roder result"));
    assert_eq!(
        response.results[0].published_at.as_deref(),
        Some("2026-05-14")
    );
    assert_eq!(response.results[0].metadata["last_updated"], "2026-05-15");
    assert_eq!(
        response.results[0].metadata["citations"],
        json!(["https://example.com/source"])
    );
    assert_eq!(response.usage.unwrap().output_tokens, Some(34));
}

#[tokio::test]
async fn domain_and_freshness_filters_are_sent_after_normalization() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "results": [{ "url": "https://example.com/page", "title": "Page" }]
        }),
    )])
    .await;
    let client = test_client(&server);
    let mut request = WebSearchRequest::new("page");
    request.max_results = 9;
    request.include_domains = vec!["https://www.Example.com/docs".to_string()];
    request.freshness = Some(Freshness::Month);
    request.country = Some("us".to_string());
    request.include_content = true;

    client.search(request).await.unwrap();
    let body = server.json_body(0).await;

    assert_eq!(body["max_results"], 9);
    assert_eq!(body["search_domain_filter"], json!(["example.com"]));
    assert_eq!(body["search_recency_filter"], "month");
    assert_eq!(body["country"], "US");
    assert!(body.get("include_content").is_none());
}

#[tokio::test]
async fn exclude_domains_are_sent_as_negative_domain_filters() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "results": [{ "url": "https://example.com/page", "title": "Page" }]
        }),
    )])
    .await;
    let client = test_client(&server);
    let mut request = WebSearchRequest::new("page");
    request.exclude_domains = vec!["Spam.test".to_string()];
    request.freshness = Some(Freshness::Day);

    client.search(request).await.unwrap();
    let body = server.json_body(0).await;

    assert_eq!(body["search_domain_filter"], json!(["-spam.test"]));
    assert_eq!(body["search_recency_filter"], "day");
}

#[tokio::test]
async fn mixed_include_and_exclude_domains_do_not_send_request() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({ "results": [{ "url": "https://example.com/unused" }] }),
    )])
    .await;
    let client = test_client(&server);
    let mut request = WebSearchRequest::new("mixed");
    request.include_domains = vec!["example.com".to_string()];
    request.exclude_domains = vec!["spam.test".to_string()];

    let error = client.search(request).await.unwrap_err().to_string();

    assert!(error.contains("either include_domains or exclude_domains"));
    assert!(server.requests().await.is_empty());
}

#[tokio::test]
async fn usage_and_request_id_are_available_in_tool_result_data() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "request_id": "req_usage",
            "results": [{ "url": "https://example.com/tool", "title": "Tool" }],
            "usage": { "searches": 2, "input_tokens": 20, "output_tokens": 40 }
        }),
    )])
    .await;
    let tool = PerplexitySearchTool::new(
        PerplexitySearchConfig::new("secret-test-key").with_base_url(server.base_url()),
    )
    .unwrap();
    let call = roder_api::tools::ToolCall {
        id: "call-1".to_string(),
        name: "perplexity_search".to_string(),
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
            effective_mode: roder_api::policy_mode::PolicyMode::Default,
        },
        call,
    )
    .await
    .unwrap();

    assert_eq!(result.name, "perplexity_search");
    assert!(result.text.contains("https://example.com/tool"));
    assert_eq!(result.data["provider"], "perplexity");
    assert_eq!(result.data["provider_request_id"], "req_usage");
    assert_eq!(result.data["usage"]["provider_metadata"]["searches"], 2);
    assert!(result.data.get("raw").is_none());
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
async fn malformed_response_is_an_error() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({ "results": [{ "title": "Missing URL" }] }),
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

fn test_client(server: &MockServer) -> PerplexitySearchClient {
    PerplexitySearchClient::new(
        PerplexitySearchConfig::new("secret-test-key").with_base_url(server.base_url()),
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
