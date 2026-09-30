//! Real pinned runtime: thread cookie isolation, observations and domain ceiling.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use roder_api::{
    policy_mode::PolicyMode,
    tools::{ToolCall, ToolContributor, ToolExecutionContext, ToolRegistry, ToolResult},
};
use roder_ext_browser_use::{BrowserUseConfig, BrowserUseServer, BrowserUseToolContributor};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn call(registry: &ToolRegistry, thread: &str, name: &str, args: Value) -> ToolResult {
    registry
        .get(name)
        .unwrap()
        .execute(
            ToolExecutionContext::new(thread, "fixture", PolicyMode::Bypass),
            ToolCall {
                id: "fixture-call".into(),
                name: name.into(),
                raw_arguments: args.to_string(),
                arguments: args,
                thread_id: thread.into(),
                turn_id: "fixture".into(),
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "starts real pinned browser-use with isolated Chrome profiles"]
async fn live_profiles_observations_and_operator_domain_scope() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let disallowed_hits = Arc::new(AtomicUsize::new(0));
    let hits = disallowed_hits.clone();
    let site = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let hits = hits.clone();
            tokio::spawn(async move {
                let mut bytes = [0; 8192];
                let length = stream.read(&mut bytes).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&bytes[..length]);
                if request.to_ascii_lowercase().contains("host: localhost:") {
                    hits.fetch_add(1, Ordering::SeqCst);
                }
                let cookie = request
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .starts_with("cookie:")
                            .then(|| line.split_once(':').unwrap().1.trim())
                    })
                    .unwrap_or("none");
                let body = format!(
                    "<!doctype html><title>MCP fixture</title><h1>MCP fixture</h1><p>Cookie: {cookie}</p><p id='count'>Count: 0</p><button onclick=\"document.querySelector('#count').textContent='Count: 1'\">Increment</button>"
                );
                let set = if request.starts_with("GET /set ") {
                    "Set-Cookie: session=thread-a; Path=/; SameSite=Lax\r\n"
                } else {
                    ""
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n{set}Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    let config = BrowserUseConfig {
        headless: true,
        allowed_domains: vec!["127.0.0.1".into()],
        ..Default::default()
    };
    let server = Arc::new(BrowserUseServer::new(&config));
    let mut registry = ToolRegistry::default();
    BrowserUseToolContributor::new(server.clone(), false)
        .contribute(&mut registry)
        .unwrap();
    let url = format!("http://127.0.0.1:{port}");
    let set = call(
        &registry,
        "a",
        "browser_use_navigate",
        json!({"url":format!("{url}/set")}),
    )
    .await;
    assert!(!set.is_error, "{}", set.text);
    let other = call(
        &registry,
        "b",
        "browser_use_navigate",
        json!({"url":format!("{url}/read")}),
    )
    .await;
    assert!(!other.is_error, "{}", other.text);
    let other_html = call(
        &registry,
        "b",
        "browser_use_get_html",
        json!({"selector":"body"}),
    )
    .await;
    assert!(
        other_html.text.contains("Cookie: none"),
        "{}",
        other_html.text
    );
    let own = call(
        &registry,
        "a",
        "browser_use_navigate",
        json!({"url":format!("{url}/read")}),
    )
    .await;
    assert!(!own.is_error, "{}", own.text);
    let own_html = call(
        &registry,
        "a",
        "browser_use_get_html",
        json!({"selector":"body"}),
    )
    .await;
    assert!(
        own_html.text.contains("session=thread-a"),
        "{}",
        own_html.text
    );
    assert_eq!(own.data["__view_image"]["detail"], "original");
    let state = call(&registry, "a", "browser_use_get_state", json!({})).await;
    let page: Value = serde_json::from_str(state.data["content"].as_str().unwrap()).unwrap();
    let index = page["interactive_elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["tag"] == "button")
        .unwrap()["index"]
        .clone();
    let clicked = call(&registry, "a", "browser_use_click", json!({"index":index})).await;
    assert!(
        !clicked.is_error && clicked.text.contains("Observed page after the action"),
        "{}",
        clicked.text
    );
    let outcome = call(
        &registry,
        "a",
        "browser_use_get_html",
        json!({"selector":"#count"}),
    )
    .await;
    assert!(outcome.text.contains("Count: 1"), "{}", outcome.text);
    assert!(clicked.text.starts_with("Browser page content"));
    let refused = call(
        &registry,
        "a",
        "browser_use_navigate",
        json!({"url":format!("http://localhost:{port}/read")}),
    )
    .await;
    assert!(
        refused.is_error && refused.text.to_ascii_lowercase().contains("block"),
        "{}",
        refused.text
    );
    assert_eq!(
        disallowed_hits.load(Ordering::SeqCst),
        0,
        "domain ceiling was checked after transmission"
    );
    server.shutdown().await;
    site.abort();
    eprintln!(
        "PASS: real pinned server, isolated thread cookies, reused own profile, observed click outcome, original screenshot, rejected off-domain request"
    );
}
