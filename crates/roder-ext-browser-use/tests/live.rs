//! Live check against the real browser-use MCP server.
//!
//! Downloads the pinned browser-use release through `uvx` on first run and
//! opens a headless browser on https://example.com. Opt in with:
//!
//! ```sh
//! cargo test -p roder-ext-browser-use --test live -- --ignored --nocapture
//! ```
//!
//! It only navigates to example.com and reads state; it never runs the
//! autonomous agent tool, signs in or submits forms.

use std::time::Duration;

use roder_ext_browser_use::{
    BrowserUseConfig, BrowserUseServer, missing_remote_tools, pinned_tools, resolve_uvx,
    server_command,
};
use roder_ext_mcp::McpStdioClient;
use serde_json::json;

fn live_config() -> BrowserUseConfig {
    // No LLM keys: the direct-control tools must work without them.
    BrowserUseConfig {
        headless: true,
        ..BrowserUseConfig::default()
    }
}

#[tokio::test]
#[ignore = "starts the real browser-use MCP server through uvx (network, browser)"]
async fn live_server_matches_the_pinned_tools_and_reads_example_com() {
    let config = live_config();
    let uvx = resolve_uvx(&config).expect("uvx on PATH");

    // Compare the live tool list with the pinned table, schemas included.
    let client = McpStdioClient::start(server_command(&config, uvx, std::env::vars()))
        .await
        .expect("server starts");
    let live = client.list_tools(Duration::from_secs(60)).await.unwrap();
    let names: Vec<_> = live.iter().map(|tool| tool.name.as_str()).collect();
    println!("live tools ({}): {}", names.len(), names.join(", "));
    assert!(
        missing_remote_tools(&live).is_empty(),
        "live server lacks pinned tools"
    );
    for pinned in pinned_tools() {
        let found = live.iter().find(|tool| tool.name == pinned.name).unwrap();
        assert_eq!(
            found.input_schema, pinned.input_schema,
            "{} schema drifted",
            pinned.name
        );
    }
    let extra: Vec<_> = live
        .iter()
        .filter(|tool| !pinned_tools().iter().any(|pinned| pinned.name == tool.name))
        .map(|tool| tool.name.as_str())
        .collect();
    assert!(
        extra.is_empty(),
        "live server offers unpinned tools: {extra:?}"
    );
    client.shutdown().await;

    // Drive it through the same path the tools use.
    let server = BrowserUseServer::new(&config);
    let navigated = server
        .call(
            "browser_navigate",
            json!({"url": "https://example.com"}),
            Duration::from_secs(120),
        )
        .await
        .expect("navigate");
    println!("navigate: {navigated}");
    assert_ne!(navigated["isError"], json!(true));

    let state = server
        .call("browser_get_state", json!({}), Duration::from_secs(120))
        .await
        .expect("get_state");
    let text = state["content"][0]["text"].as_str().unwrap_or_default();
    println!("state: {text}");
    assert!(text.contains("example.com"), "{text}");
    server.shutdown().await;
}
