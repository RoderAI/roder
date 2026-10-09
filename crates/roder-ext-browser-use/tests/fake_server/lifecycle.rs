//! Starting, reusing, stopping and cancelling the server and its browser.

use std::path::PathBuf;
use std::sync::Arc;

use roder_ext_browser_use::{BrowserUseConfig, BrowserUseServer, UNTRUSTED_NOTE, server_command};
use serde_json::{Value, json};

#[cfg(unix)]
use crate::support::alive;
use crate::support::{
    OPENAI_KEY, fake_launch, keyed_config, registry_for, run, run_thread, state_json,
};

#[tokio::test]
async fn tools_drive_the_server_with_a_scoped_environment() {
    let server = fake_launch(keyed_config(), None);
    let registry = registry_for(server.clone(), true);

    let navigated = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com"}),
    )
    .await;
    assert!(!navigated.is_error, "{}", navigated.text);
    assert!(navigated.text.contains("Navigated to: https://example.com"));
    assert!(navigated.text.contains("Observed page after the action"));
    assert!(navigated.text.contains("Example Domain"));
    assert_eq!(navigated.data["__view_image"]["detail"], "original");

    let state = run(&registry, "browser_use_get_state", json!({})).await;
    assert!(state.text.starts_with(UNTRUSTED_NOTE), "{}", state.text);
    assert_eq!(state.data["untrusted"], json!(true));
    let state = state_json(&state.text);
    assert_eq!(state["headless"], "false");
    assert_eq!(state["disable_security"], Value::Null);
    assert_eq!(state["has_openai_key"], true);
    assert_eq!(state["leaked_github_token"], Value::Null);
    assert_eq!(state["ping_answered"], true);

    let html = run(&registry, "browser_use_get_html", json!({})).await;
    assert!(!html.text.contains(OPENAI_KEY), "{}", html.text);
    assert!(
        html.text.contains("<html>[redacted]</html>"),
        "{}",
        html.text
    );
    assert!(!html.data.to_string().contains(OPENAI_KEY));

    let agent = run(
        &registry,
        "browser_use_agent",
        json!({"task": "read the heading"}),
    )
    .await;
    assert!(!agent.is_error, "{}", agent.text);
    server.shutdown().await;
}

#[tokio::test]
async fn without_a_key_llm_tools_fail_clearly_and_direct_tools_work() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);

    let navigated = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com"}),
    )
    .await;
    assert!(!navigated.is_error, "{}", navigated.text);
    let state = run(&registry, "browser_use_get_state", json!({})).await;
    assert_eq!(state_json(&state.text)["has_openai_key"], false);

    for name in ["browser_use_extract_content", "browser_use_agent"] {
        let result = run(&registry, name, json!({"query": "title", "task": "x"})).await;
        assert!(result.is_error, "{name}");
        assert!(result.text.contains("OPENAI_API_KEY"), "{}", result.text);
    }
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_stops_the_server_and_its_browser() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    let state = run(&registry, "browser_use_get_state", json!({})).await;
    let browser_pid = state_json(&state.text)["browser_pid"].as_u64().unwrap();
    assert!(alive(browser_pid));

    server.shutdown().await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while alive(browser_pid) && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        !alive(browser_pid),
        "stand-in browser {browser_pid} outlived the server"
    );

    // The next call starts a fresh server.
    let again = run(&registry, "browser_use_list_tabs", json!({})).await;
    assert!(!again.is_error, "{}", again.text);
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_the_server_stops_its_browser() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    let state = run(&registry, "browser_use_get_state", json!({})).await;
    let browser_pid = state_json(&state.text)["browser_pid"].as_u64().unwrap();
    drop(registry);
    drop(server);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while alive(browser_pid) && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        !alive(browser_pid),
        "stand-in browser {browser_pid} outlived Roder's handle"
    );
}

#[tokio::test]
async fn a_server_missing_a_tool_is_refused() {
    let server = fake_launch(BrowserUseConfig::default(), Some("browser_click"));
    let registry = registry_for(server.clone(), false);
    let result = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com"}),
    )
    .await;
    assert!(result.is_error);
    assert!(
        result.text.contains("does not offer browser_click"),
        "{}",
        result.text
    );
}

#[tokio::test]
async fn a_server_that_fails_to_start_is_reported() {
    let server = Arc::new(BrowserUseServer::with_launch(
        "browser-use[cli]==0.13.10".into(),
        Arc::new(|| {
            let mut spec =
                server_command(&keyed_config(), PathBuf::from("/bin/sh"), std::env::vars());
            spec.args = vec![
                "-c".into(),
                format!("echo 'boom: bad key {OPENAI_KEY}' >&2; exit 3"),
            ];
            Ok(spec)
        }),
    ));
    let registry = registry_for(server, true);
    let result = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com"}),
    )
    .await;
    assert!(result.is_error);
    assert!(result.text.contains("failed to start"), "{}", result.text);
    assert!(result.text.contains("boom"), "{}", result.text);
    assert!(!result.text.contains(OPENAI_KEY), "{}", result.text);
}

#[tokio::test]
async fn threads_own_distinct_browser_processes_and_reuse_their_own_session() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    let first = run_thread(&registry, "first", "browser_use_get_state", json!({})).await;
    let second = run_thread(&registry, "second", "browser_use_get_state", json!({})).await;
    let again = run_thread(&registry, "first", "browser_use_get_state", json!({})).await;
    let pid = |result: &roder_api::tools::ToolResult| {
        state_json(&result.text)["browser_pid"].as_u64().unwrap()
    };
    assert_ne!(pid(&first), pid(&second));
    assert_eq!(pid(&first), pid(&again));
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_a_tool_stops_browser_work_and_next_call_starts_fresh() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = Arc::new(registry_for(server.clone(), false));
    let state = run(&registry, "browser_use_get_state", json!({})).await;
    let pid = state_json(&state.text)["browser_pid"].as_u64().unwrap();
    let running_registry = registry.clone();
    let task = tokio::spawn(async move {
        run(&running_registry, "browser_use_click", json!({"index":999})).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!task.is_finished());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while alive(pid) && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(!alive(pid), "Cancelled tool left browser running");
    let fresh = run(&registry, "browser_use_get_state", json!({})).await;
    assert_ne!(
        state_json(&fresh.text)["browser_pid"].as_u64().unwrap(),
        pid
    );
    assert!(
        fresh.text.contains("ran in a fresh browser"),
        "a cancelled call's replacement browser is announced: {}",
        fresh.text
    );
    server.shutdown().await;
}
