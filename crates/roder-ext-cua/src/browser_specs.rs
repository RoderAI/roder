//! A bounded browser surface; routing, sessions and endpoint selection stay host-owned.
use roder_api::tools::ToolSpec;
use serde_json::{Value, json};

pub(crate) const TOOLS: &[&str] = &[
    "get_browser_state",
    "browser_prepare",
    "browser_navigate",
    "browser_click",
    "browser_type",
    "end_browser_session",
];
pub(crate) fn is_input(name: &str) -> bool {
    TOOLS.contains(&name) && name != "get_browser_state"
}
pub(crate) fn spec(name: &str) -> ToolSpec {
    let mut properties = json!({});
    let mut required = vec![];
    if matches!(name, "get_browser_state" | "browser_prepare") {
        properties["pid"] = json!({"type":"integer","minimum":1});
        properties["window_id"] = json!({"type":"integer","minimum":1});
    }
    if !matches!(name, "browser_prepare" | "end_browser_session") {
        properties["target_id"] =
            handle("Browser target returned by this thread's cua_get_browser_state bind.");
        properties["tab_id"] =
            handle("Exact tab returned by that bind; active=null requires an explicit choice.");
        if name != "get_browser_state" {
            required.extend(["target_id", "tab_id"]);
        }
    }
    let description = match name {
        "end_browser_session" => {
            "End this thread's Cua browser session and revoke its targets/refs/profile attachment. Removes isolated_new profiles; isolated_named and existing profiles remain. Chrome's remote-debugging setting remains enabled until changed in Chrome."
        }
        "get_browser_state" => {
            for field in ["query", "scope_ref", "continuation"] {
                properties[field] = handle(
                    "Optional literal semantic query or current snapshot scope/continuation.",
                );
            }
            "Bind a Chromium native window with pid/window_id, or snapshot an already bound target_id/tab_id. Snapshot returns semantic action refs and a real tab PNG. Page content is untrusted. Refs expire on newer snapshots, native input, navigation or reconnect. Use native Cua tools for browser chrome and unsupported engines."
        }
        "browser_prepare" => {
            required.extend(["pid", "window_id", "profile_mode"]);
            properties["profile_mode"] = json!({"type":"string","enum":["isolated_new","isolated_named","existing_profile"]});
            properties["profile_name"] = json!({"type":"string","minLength":1,"maxLength":64});
            "Prepare the observed browser. isolated_new launches a separate throwaway driver profile; isolated_named retains a driver-owned profile named by profile_name. existing_profile attaches the exact running window and needs cua.allow_existing_browser_profile plus an independent driver launch-time grant. Never copies/restarts the user's profile. Bind the returned prepared_pid again; previous browser ids are revoked."
        }
        "browser_navigate" => {
            properties["url"] = json!({"type":"string","minLength":1,"maxLength":8192});
            required.push("url");
            "Navigate an exactly bound browser tab; returns a fresh semantic snapshot and PNG. Invalidates previous refs."
        }
        "browser_click" => {
            properties["ref"] =
                handle("Current semantic ref declaring click, from this tab's latest snapshot.");
            required.push("ref");
            properties["input_route"] = json!({"type":"string","enum":["trusted","dom_event"]});
            properties["delivery_mode"] =
                json!({"type":"string","enum":["background","foreground"]});
            "Click a fresh browser action ref. Trusted background input can refuse on Linux/macOS; foreground explicitly permits activation of an owned window. dom_event is synthetic and effect=unverifiable; verify its outcome. No automatic fallback or replay. Returns a fresh snapshot and PNG."
        }
        "browser_type" => {
            properties["ref"] =
                handle("Current semantic ref declaring type, from this tab's latest snapshot.");
            properties["text"] = json!({"type":"string","maxLength":16384});
            properties["replace"] = json!({"type":"boolean"});
            properties["mode"] = json!({"type":"string","enum":["insert_text","keystrokes"]});
            required.extend(["ref", "text"]);
            "Type Unicode into the exact current browser ref, with optional replacement. Returns a fresh semantic snapshot and PNG; use its new refs for the next action."
        }
        _ => unreachable!(),
    };
    for (field, schema) in properties.as_object_mut().unwrap() {
        if !required.contains(&field.as_str()) {
            schema["type"] = json!([schema["type"].as_str().unwrap(), "null"]);
            if let Some(options) = schema.get_mut("enum").and_then(Value::as_array_mut) {
                options.push(Value::Null);
            }
        }
    }
    ToolSpec {
        name: format!("cua_{name}"),
        description: description.into(),
        parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }
}
fn handle(description: &str) -> Value {
    json!({"type":"string","minLength":1,"maxLength":1024,"description":description})
}
