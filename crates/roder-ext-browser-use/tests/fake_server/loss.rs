//! What the model is told when a call takes the browser down, and the calls
//! that must not: a healthy server turning a call down keeps its browser.

use std::path::PathBuf;

use roder_ext_browser_use::BrowserUseConfig;
use serde_json::json;

#[cfg(unix)]
use crate::support::alive;
use crate::support::{fake_launch, registry_for, run, state_json};

/// Phrases only the error of a call that dropped the browser contains.
const LOSS_WORDS: [&str; 4] = [
    "browser-use browser was shut down",
    "logins and cookies are gone",
    "Nothing was restarted",
    "starts a fresh browser",
];

#[cfg(unix)]
#[tokio::test]
async fn a_killed_client_says_the_browser_is_gone_and_the_next_call_starts_fresh() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    let first = run(&registry, "browser_use_get_state", json!({})).await;
    let old_pid = state_json(&first.text)["browser_pid"].as_u64().unwrap();

    let lost = run(&registry, "browser_use_click", json!({"index": 777})).await;
    assert!(lost.is_error, "{}", lost.text);
    for expected in [
        "exited during tools/call",
        "browser-use browser was shut down",
        "logins and cookies are gone",
        "The next browser_use call starts a fresh browser",
    ] {
        assert!(lost.text.contains(expected), "{expected}: {}", lost.text);
    }

    // Nothing was restarted silently: the fresh browser is announced.
    let fresh = run(&registry, "browser_use_get_state", json!({})).await;
    assert!(!fresh.is_error, "{}", fresh.text);
    assert!(
        fresh.text.contains("ran in a fresh browser"),
        "{}",
        fresh.text
    );
    let new_pid = state_json(&fresh.text)["browser_pid"].as_u64().unwrap();
    assert_ne!(new_pid, old_pid);

    // The notice is delivered once.
    let again = run(&registry, "browser_use_get_state", json!({})).await;
    assert!(!again.text.contains("fresh browser"), "{}", again.text);
    assert_eq!(state_json(&again.text)["browser_pid"], new_pid);
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_timed_out_call_says_the_browser_is_gone() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let error = server
        .call(
            "browser_click",
            json!({"index": 999}),
            std::time::Duration::from_millis(300),
        )
        .await
        .unwrap_err();
    let text = format!("{error:#}");
    for expected in [
        "did not answer",
        "browser-use browser was shut down",
        "logins and cookies are gone",
        "The next browser_use call starts a fresh browser",
    ] {
        assert!(text.contains(expected), "{expected}: {text}");
    }
    let fresh = server
        .call(
            "browser_get_state",
            json!({}),
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap();
    assert!(
        fresh["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("ran in a fresh browser"),
        "{fresh}"
    );
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_json_rpc_error_from_a_healthy_server_keeps_the_browser_and_says_nothing_was_lost() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    let first = run(&registry, "browser_use_get_state", json!({})).await;
    let before = state_json(&first.text);
    let pid = before["browser_pid"].as_u64().unwrap();
    let profile = PathBuf::from(before["config_dir"].as_str().expect("the owned profile"));
    assert!(profile.is_dir());

    // The server answers this call with a JSON-RPC error and runs nothing.
    let rejected = run(&registry, "browser_use_click", json!({"index": 555})).await;
    assert!(rejected.is_error, "{}", rejected.text);
    assert!(
        rejected.text.contains("MCP error -32602"),
        "the server's own error is shown: {}",
        rejected.text
    );
    for words in LOSS_WORDS {
        assert!(
            !rejected.text.contains(words),
            "the browser was not lost, so the error must not say so ({words}): {}",
            rejected.text
        );
    }
    assert!(alive(pid), "the rejected call stopped the browser");
    assert!(profile.is_dir(), "the rejected call removed the profile");

    // The next calls run in the same browser, with no fresh-browser notice.
    let next = run(&registry, "browser_use_get_state", json!({})).await;
    assert!(!next.is_error, "{}", next.text);
    assert!(!next.text.contains("fresh browser"), "{}", next.text);
    let after = state_json(&next.text);
    assert_eq!(after["browser_pid"], pid);
    assert_eq!(after["config_dir"], before["config_dir"]);
    assert_eq!(
        after["state_calls"], 2,
        "the same server process answered both states"
    );
    let click = run(&registry, "browser_use_click", json!({"index": 3})).await;
    assert!(!click.is_error, "{}", click.text);
    assert!(click.text.contains("browser_click ok"), "{}", click.text);
    assert!(!click.text.contains("fresh browser"), "{}", click.text);
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn an_error_reply_to_the_observation_after_an_action_still_takes_the_browser_down() {
    // The action ran; the server then failed to read the page. That is a sick
    // server, not a call the model got wrong, so the browser goes and the
    // error says so.
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    let first = run(&registry, "browser_use_get_state", json!({})).await;
    let old_pid = state_json(&first.text)["browser_pid"].as_u64().unwrap();

    let lost = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/unreadable"}),
    )
    .await;
    assert!(lost.is_error, "{}", lost.text);
    assert!(lost.text.contains("MCP error -32603"), "{}", lost.text);
    for words in LOSS_WORDS {
        assert!(lost.text.contains(words), "{words}: {}", lost.text);
    }

    let fresh = run(&registry, "browser_use_get_state", json!({})).await;
    assert!(!fresh.is_error, "{}", fresh.text);
    assert!(
        fresh.text.contains("ran in a fresh browser"),
        "{}",
        fresh.text
    );
    assert_ne!(state_json(&fresh.text)["browser_pid"], old_pid);
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_reply_whose_error_is_not_a_json_rpc_error_object_takes_the_browser_down() {
    // `"error": null` has the shape of an answer, but the server did not
    // decline the call in the protocol's words, so nothing says the browser is
    // as it was. It is an unusable reply, and the browser goes.
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    let first = run(&registry, "browser_use_get_state", json!({})).await;
    let old_pid = state_json(&first.text)["browser_pid"].as_u64().unwrap();

    let lost = run(&registry, "browser_use_click", json!({"index": 556})).await;
    assert!(lost.is_error, "{}", lost.text);
    for words in LOSS_WORDS {
        assert!(lost.text.contains(words), "{words}: {}", lost.text);
    }

    let fresh = run(&registry, "browser_use_get_state", json!({})).await;
    assert!(!fresh.is_error, "{}", fresh.text);
    assert!(
        fresh.text.contains("ran in a fresh browser"),
        "{}",
        fresh.text
    );
    assert_ne!(state_json(&fresh.text)["browser_pid"], old_pid);
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn an_observation_without_content_takes_the_browser_down() {
    // The action ran and the server answered the state with a result that
    // carries no content. The observation is the evidence the report is
    // checked against, so an apparently successful action next to nothing is
    // not returned.
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    let first = run(&registry, "browser_use_get_state", json!({})).await;
    let old_pid = state_json(&first.text)["browser_pid"].as_u64().unwrap();

    let lost = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/no-content"}),
    )
    .await;
    assert!(lost.is_error, "{}", lost.text);
    assert!(
        lost.text.contains("no observation content"),
        "{}",
        lost.text
    );
    for words in LOSS_WORDS {
        assert!(lost.text.contains(words), "{words}: {}", lost.text);
    }

    let fresh = run(&registry, "browser_use_get_state", json!({})).await;
    assert!(!fresh.is_error, "{}", fresh.text);
    assert!(
        fresh.text.contains("ran in a fresh browser"),
        "{}",
        fresh.text
    );
    assert_ne!(state_json(&fresh.text)["browser_pid"], old_pid);
    server.shutdown().await;
}
