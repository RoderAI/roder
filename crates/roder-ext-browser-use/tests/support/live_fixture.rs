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
                    "<!doctype html><title>MCP fixture</title><h1>MCP fixture</h1><p>Cookie: {cookie}</p><p id='count'>Count: 0</p><button onclick=\"document.querySelector('#count').textContent='Count: 1'\">Increment</button><select id='pick' onchange=\"document.querySelector('#count').textContent='Picked: '+this.value\"><option value=''>Pick one</option><option value='b'>Beta</option></select>"
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
    // The real server's pretty-printed state must come back as the compact
    // view, one `[index] tag ...` line per element. A pin bump that changes
    // the state's shape would fall back to raw JSON and fail here.
    let state = call(&registry, "a", "browser_use_get_state", json!({})).await;
    let view = state.data["content"].as_str().unwrap();
    assert!(
        view.contains("interactive elements 1-") && !view.contains("\"interactive_elements\""),
        "the real state was not compacted:\n{view}"
    );
    let index: u64 = view
        .lines()
        .find_map(|line| {
            let (index, rest) = line.strip_prefix('[')?.split_once("] ")?;
            rest.starts_with("button ").then(|| index.parse().ok())?
        })
        .unwrap_or_else(|| panic!("no button line in:\n{view}"));
    // The real server lists a native select with the tag `select`, which is
    // what the wrapper's refusal reads. A pin bump that spells it otherwise
    // fails here.
    let select: u64 = view
        .lines()
        .find_map(|line| {
            let (index, rest) = line.strip_prefix('[')?.split_once("] ")?;
            rest.starts_with("select ").then(|| index.parse().ok())?
        })
        .unwrap_or_else(|| panic!("no select line in:\n{view}"));
    // Paging past the end is answered by the wrapper, not the real server.
    let past = call(
        &registry,
        "a",
        "browser_use_get_state",
        json!({"offset": 500}),
    )
    .await;
    assert!(
        !past.is_error && past.text.contains("none at offset 500"),
        "{}",
        past.text
    );
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
    // The pinned server reports a stale index as plain text, not as an
    // error. Roder's exact-string classification depends on this wording, so
    // a pin bump that changes it fails here.
    let stale = call(&registry, "a", "browser_use_click", json!({"index":987654})).await;
    assert!(
        stale.is_error && stale.text.contains("Element with index 987654 not found"),
        "{}",
        stale.text
    );
    assert!(stale.text.contains("Observed page after the action"));
    // The server would answer both of these with success and leave the select
    // as it was, so Roder refuses them and the page is left alone.
    for (tool, args) in [
        ("browser_use_click", json!({"index": select})),
        ("browser_use_type", json!({"index": select, "text": "Beta"})),
    ] {
        let refused = call(&registry, "a", tool, args).await;
        assert!(
            refused.is_error && refused.text.contains("is a native <select>"),
            "{tool}: {}",
            refused.text
        );
    }
    let untouched = call(
        &registry,
        "a",
        "browser_use_get_html",
        json!({"selector":"#count"}),
    )
    .await;
    assert!(untouched.text.contains("Count: 1"), "{}", untouched.text);
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
        "PASS: real pinned server, isolated thread cookies, reused own profile, observed click outcome, original screenshot, refused select, rejected off-domain request"
    );
}
