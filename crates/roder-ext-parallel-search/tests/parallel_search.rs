use std::collections::VecDeque;
use std::sync::Arc;

use roder_ext_parallel_search::client::{ParallelSearchConfig, ParallelSearchOptions};
use roder_ext_parallel_search::{
    ParallelExtractRequest, ParallelExtractTool, ParallelSearchClient, ParallelSearchTool,
};
use roder_web_search::WebSearchRequest;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[tokio::test]
async fn objective_only_search_derives_queries_and_normalizes_excerpts() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "search_id": "search_basic",
            "results": [{
                "url": "https://example.com/roder",
                "title": "Roder",
                "excerpts": ["Parallel excerpt"],
                "publish_date": "2026-05-15",
                "score": 0.92
            }]
        }),
    )])
    .await;
    let client = test_client(&server);

    let response = client
        .search(
            WebSearchRequest::new("Roder web search extension design"),
            ParallelSearchOptions::default(),
        )
        .await
        .unwrap();
    let body = server.json_body(0).await;

    assert_eq!(server.requests().await.len(), 1);
    assert_eq!(body["objective"], "Roder web search extension design");
    assert_eq!(body["search_queries"].as_array().unwrap().len(), 3);
    assert_eq!(body["mode"], "advanced");
    assert_eq!(body["advanced_settings"]["max_results"], 5);
    assert!(body.get("max_results").is_none());
    assert_eq!(response.provider, "parallel");
    assert_eq!(response.results[0].url, "https://example.com/roder");
    assert_eq!(
        response.results[0].snippet.as_deref(),
        Some("Parallel excerpt")
    );
    assert_eq!(
        response.results[0].published_at.as_deref(),
        Some("2026-05-15")
    );
}

#[tokio::test]
async fn provider_search_queries_are_sent_unchanged() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "results": [{ "url": "https://example.com/query", "summary": "Query result" }]
        }),
    )])
    .await;
    let client = test_client(&server);
    let options = ParallelSearchOptions {
        search_queries: vec!["first query".to_string(), "second query".to_string()],
    };

    client
        .search(WebSearchRequest::new("objective"), options)
        .await
        .unwrap();
    let body = server.json_body(0).await;

    assert_eq!(body["objective"], "objective");
    assert_eq!(
        body["search_queries"],
        json!(["first query", "second query"])
    );
    assert_eq!(body["advanced_settings"]["max_results"], 5);
}

#[tokio::test]
async fn advanced_settings_carry_domains_and_location() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "results": [{ "url": "https://example.com/filtered", "excerpts": ["A", "B"] }]
        }),
    )])
    .await;
    let client = test_client(&server);
    let mut request = WebSearchRequest::new("filter me");
    request.max_results = 3;
    request.include_domains = vec!["example.com".to_string()];
    request.exclude_domains = vec!["spam.example".to_string()];
    request.country = Some("US".to_string());

    let response = client
        .search(request, ParallelSearchOptions::default())
        .await
        .unwrap();
    let body = server.json_body(0).await;

    assert!(body.get("include_domains").is_none());
    assert!(body.get("country").is_none());
    assert_eq!(body["advanced_settings"]["max_results"], 3);
    assert_eq!(
        body["advanced_settings"]["source_policy"]["include_domains"],
        json!(["example.com"])
    );
    assert_eq!(
        body["advanced_settings"]["source_policy"]["exclude_domains"],
        json!(["spam.example"])
    );
    assert_eq!(body["advanced_settings"]["location"], "us");
    assert_eq!(
        response.results[0].snippet.as_deref(),
        Some("A\n\nB")
    );
}

#[tokio::test]
async fn object_warnings_and_array_usage_are_normalized() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "search_id": "search_usage",
            "results": [{ "url": "https://example.com/usage", "title": "Usage" }],
            "warnings": [{ "type": "warning", "message": "partial corpus" }],
            "usage": [{ "name": "sku_search", "count": 2 }]
        }),
    )])
    .await;
    let client = test_client(&server);

    let response = client
        .search(
            WebSearchRequest::new("usage"),
            ParallelSearchOptions::default(),
        )
        .await
        .unwrap();

    assert_eq!(response.warnings, vec!["partial corpus".to_string()]);
    assert_eq!(response.usage.as_ref().and_then(|u| u.requests), Some(2));
}

#[tokio::test]
async fn warnings_and_request_id_are_available_in_tool_result_data() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "search_id": "search_tool",
            "results": [{ "url": "https://example.com/tool", "title": "Tool" }],
            "warnings": ["partial corpus"],
            "usage": { "requests": 1 }
        }),
    )])
    .await;
    let tool = ParallelSearchTool::new(
        ParallelSearchConfig::new("secret-test-key").with_base_url(server.base_url()),
    )
    .unwrap();
    let call = roder_api::tools::ToolCall {
        id: "call-1".to_string(),
        name: "parallel_search".to_string(),
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

    assert_eq!(result.name, "parallel_search");
    assert!(result.text.contains("https://example.com/tool"));
    assert!(result.text.contains("Warnings:"));
    assert_eq!(result.data["provider"], "parallel");
    assert_eq!(result.data["provider_request_id"], "search_tool");
    assert_eq!(result.data["warnings"], json!(["partial corpus"]));
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
        .search(
            WebSearchRequest::new("auth"),
            ParallelSearchOptions::default(),
        )
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
        .search(
            WebSearchRequest::new("limits"),
            ParallelSearchOptions::default(),
        )
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
        .search(
            WebSearchRequest::new("bad response"),
            ParallelSearchOptions::default(),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("usable results"));
}

#[tokio::test]
async fn extract_sends_urls_objective_and_full_content_settings() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "extract_id": "extract_1",
            "session_id": "session_1",
            "results": [{
                "url": "https://example.com/page",
                "title": "Example",
                "publish_date": "2026-01-02",
                "excerpts": ["Focused excerpt"],
                "full_content": "# Full page"
            }],
            "errors": [],
            "usage": [{ "name": "sku_extract_excerpts", "count": 1 }]
        }),
    )])
    .await;
    let client = test_client(&server);
    let request = ParallelExtractRequest {
        urls: vec!["https://example.com/page".to_string()],
        objective: Some("Summarize the page".to_string()),
        search_queries: vec!["example page".to_string()],
        max_chars_total: Some(1200),
        session_id: Some("session_prior".to_string()),
        full_content: true,
        max_chars_per_result: Some(400),
    };

    let response = client.extract(request).await.unwrap();
    let body = server.json_body(0).await;

    assert_eq!(body["urls"], json!(["https://example.com/page"]));
    assert_eq!(body["objective"], "Summarize the page");
    assert_eq!(body["search_queries"], json!(["example page"]));
    assert_eq!(body["max_chars_total"], 1200);
    assert_eq!(body["session_id"], "session_prior");
    assert_eq!(
        body["advanced_settings"]["full_content"]["max_chars_per_result"],
        400
    );
    assert_eq!(response.extract_id.as_deref(), Some("extract_1"));
    assert_eq!(response.session_id.as_deref(), Some("session_1"));
    assert_eq!(response.results[0].title.as_deref(), Some("Example"));
    assert_eq!(
        response.results[0].excerpts,
        vec!["Focused excerpt".to_string()]
    );
    assert_eq!(
        response.results[0].full_content.as_deref(),
        Some("# Full page")
    );
    assert_eq!(response.usage.as_ref().and_then(|u| u.requests), Some(1));
}

#[tokio::test]
async fn extract_tool_accepts_single_url_alias_and_renders_text() {
    let server = MockServer::start(vec![MockResponse::json(
        200,
        json!({
            "extract_id": "extract_tool",
            "results": [{
                "url": "https://example.com/tool",
                "title": "Tool Page",
                "excerpts": ["Tool excerpt"]
            }],
            "errors": [{
                "url": "https://example.com/missing",
                "message": "not found"
            }]
        }),
    )])
    .await;
    let tool = ParallelExtractTool::new(
        ParallelSearchConfig::new("secret-test-key").with_base_url(server.base_url()),
    )
    .unwrap();
    let call = roder_api::tools::ToolCall {
        id: "call-extract".to_string(),
        name: "parallel_extract".to_string(),
        arguments: json!({
            "url": "https://example.com/tool",
            "query": "What is on the page?"
        }),
        raw_arguments: r#"{"url":"https://example.com/tool","query":"What is on the page?"}"#
            .to_string(),
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
    let body = server.json_body(0).await;

    assert_eq!(result.name, "parallel_extract");
    assert!(!result.is_error);
    assert!(result.text.contains("https://example.com/tool"));
    assert!(result.text.contains("Tool excerpt"));
    assert!(result.text.contains("not found"));
    assert_eq!(body["urls"], json!(["https://example.com/tool"]));
    assert_eq!(body["objective"], "What is on the page?");
    assert_eq!(result.data["provider"], "parallel");
    assert_eq!(result.data["provider_request_id"], "extract_tool");
}

#[tokio::test]
async fn extract_rejects_empty_urls() {
    let error = ParallelExtractRequest::from_tool_arguments(&json!({ "objective": "x" }))
        .unwrap_err()
        .to_string();
    assert!(error.contains("at least one url"));
}

fn test_client(server: &MockServer) -> ParallelSearchClient {
    ParallelSearchClient::new(
        ParallelSearchConfig::new("secret-test-key").with_base_url(server.base_url()),
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
