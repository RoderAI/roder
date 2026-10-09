//! Native computer batches against real headless Chrome: the facts a batch
//! returns, the screenshot it never loses, Mac editing chords, and the stop
//! after a navigation. Chrome is required only for the browser tests; they
//! skip when it is absent unless `RODER_REQUIRE_CHROME=1`.
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use roder_api::{
    computer::ComputerActions,
    policy_mode::PolicyMode,
    tools::{ToolCall, ToolExecutionContext, ToolExecutor},
    transcript::VIEW_IMAGE_DISPLAY_KEY,
};
use roder_ext_chrome::{
    ComputerTool,
    direct::{
        DirectBinding, DirectGuard, DirectLease, DirectSession, DirectStep, DirectTab, OpenGuard,
        tool_result,
    },
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio_tungstenite::tungstenite::Message;

#[path = "../examples/native_computer/support.rs"]
#[allow(dead_code)]
mod support;

fn actions(list: Value) -> ComputerActions {
    serde_json::from_value(json!({ "actions": list })).unwrap()
}

fn notes(data: &Value) -> Vec<String> {
    data["computer_notes"]
        .as_array()
        .map(|notes| {
            notes
                .iter()
                .map(|note| note.as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The first page of the browser: its page websocket and target id.
async fn first_page(browser: &support::Browser) -> (String, String) {
    let pages: Value = reqwest::get(format!("{}/json", browser.endpoint))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let page = pages
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["type"] == "page")
        .unwrap();
    (
        page["webSocketDebuggerUrl"].as_str().unwrap().into(),
        page["id"].as_str().unwrap().into(),
    )
}

// The tests are grouped by behaviour in `native_notes/`; they share this file's
// helpers and imports, so they are included here, not compiled as modules (a
// test keeps the name it had when the groups were one file).
include!("native_notes/mac_chords.rs");
include!("native_notes/traps.rs");
include!("native_notes/lost_screenshot.rs");
