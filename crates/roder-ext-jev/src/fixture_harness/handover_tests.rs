//! The hand-over tools (`jev_tab_*`) on a thread's Jev tab, and the full
//! browser tools' handling of secrets, on real pages.

use std::sync::Arc;

use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolExecutor, ToolResult};
use roder_api::transcript::VIEW_IMAGE_DISPLAY_KEY;
use roder_ext_chrome::direct::{DirectGuard, DirectSession, DirectTab};
use serde_json::{Value, json};

use super::fallback_script::ScriptedFallback;
use super::fallback_tests::{ceilings, jev};
use super::sessions::{call_falling_back, targets, test_sessions};
use crate::fallback::FallbackMode;
use crate::fallback::guard::JevGuard;
use crate::fallback::handover::{NO_JEV_TAB, tools_on};
use crate::scope::JevOriginScope;
use crate::secret::Secrets;
use crate::session::JevSessions;

async fn run_tool(
    tools: &[Arc<dyn ToolExecutor>],
    thread: &str,
    name: &str,
    args: Value,
) -> ToolResult {
    let tool = tools
        .iter()
        .find(|tool| tool.spec().name == name)
        .unwrap_or_else(|| panic!("no tool {name}"));
    tool.execute(
        ToolExecutionContext::new(thread, "turn", PolicyMode::Bypass),
        ToolCall {
            id: "call".into(),
            name: name.into(),
            raw_arguments: args.to_string(),
            arguments: args,
            thread_id: thread.into(),
            turn_id: "turn".into(),
        },
    )
    .await
    .unwrap()
}

fn ref_of(result: &ToolResult, label: &str) -> Value {
    result.data["page"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["label"] == label)
        .unwrap_or_else(|| panic!("no {label}: {}", result.text))["ref"]
        .clone()
}

/// After a hand-over, the calling model goes on with the full tools: they
/// drive the same tab (Chrome's tab count does not move), record where they
/// left it, and the thread's next Jev call goes on from there.
#[tokio::test]
async fn the_caller_goes_on_in_the_same_tab_with_the_full_tools() {
    let harness = harness_or_skip!();
    let sessions: &'static JevSessions = Box::leak(Box::new(test_sessions()));
    let tools = tools_on(sessions);
    let before = harness.page_targets().await;
    let handed = call_falling_back(
        &harness,
        sessions,
        "caller",
        json!({"goal": "open laptops", "url": harness.site.url("hover-menu.html")}),
        jev(json!([{"click": "Products", "repeat": 3}])),
        Arc::new(ScriptedFallback::from_json(json!([]))),
        &ceilings(FallbackMode::Handover),
    )
    .await
    .unwrap();
    assert_eq!(handed["fallback"]["ran"], false, "{handed:#}");
    let tab = targets(sessions, "caller");

    let look = run_tool(&tools, "caller", "jev_tab_look", json!({})).await;
    assert!(!look.is_error, "{}", look.text);
    assert!(
        look.text.contains("untrusted page content"),
        "{}",
        look.text
    );
    let products = ref_of(&look, "Products");
    let hover = run_tool(&tools, "caller", "jev_tab_hover", json!({"ref": products})).await;
    assert!(hover.text.contains("Laptops"), "{}", hover.text);
    // A ref read before an action still names the same element after it.
    assert_eq!(ref_of(&hover, "Products"), products);
    let laptops = ref_of(&hover, "Laptops");
    // Omitted coordinates are null; (0,0) is a valid viewport point.
    let nowhere = run_tool(
        &tools,
        "caller",
        "jev_tab_click",
        json!({"ref": "", "x": null, "y": null, "button": "", "double": false}),
    )
    .await;
    assert!(nowhere.is_error, "{}", nowhere.text);
    let click = run_tool(
        &tools,
        "caller",
        "jev_tab_click",
        json!({"ref": laptops, "x": null, "y": null, "button": "", "double": false,
            "authorize_irreversible": false}),
    )
    .await;
    assert!(!click.is_error, "{}", click.text);
    assert!(click.text.contains("page=laptops"), "{}", click.text);
    assert_eq!(harness.settled_page_targets(before + 1).await, before + 1);
    assert_eq!(targets(sessions, "caller"), tab);

    let next = call_falling_back(
        &harness,
        sessions,
        "caller",
        json!({"goal": "back to basics", "url": ""}),
        jev(json!([{"click": "Back to basics"}])),
        Arc::new(ScriptedFallback::from_json(json!([]))),
        &ceilings(FallbackMode::Handover),
    )
    .await
    .unwrap();
    assert_eq!(next["session"]["tab_note"], "continued", "{next:#}");
    assert_eq!(next["status"], "done");
    assert_eq!(next["session"]["totals"]["tab_tool_calls"], 4);

    // A thread with no Jev tab cannot reach any other tab.
    let none = run_tool(&tools, "no-tab", "jev_tab_look", json!({})).await;
    assert!(none.is_error);
    assert_eq!(none.text, NO_JEV_TAB);
}

/// What a secret field holds is never read; what the tools type there is
/// reported as [secret] and scrubbed wherever the page shows it; a
/// screenshot blacks out filled secret fields, and is withheld while the
/// page shows a typed secret.
#[tokio::test]
async fn secrets_stay_out_of_reads_and_screenshots() {
    let harness = harness_or_skip!();
    let page = harness.open("secret-echo.html").await.unwrap();
    let tab = DirectTab::Target {
        endpoint: harness.endpoint().to_string(),
        target_id: page.target_id().to_string(),
    };
    let guard = Arc::new(JevGuard::new(
        JevOriginScope::any(),
        false,
        true,
        Secrets::default(),
    ));
    let mut session = DirectSession::attach(&tab, guard.clone(), false)
        .await
        .unwrap();
    let look = session.run("look", &json!({})).await;
    let field = look.data["page"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["secret"] == true)
        .unwrap()["ref"]
        .clone();
    let secret = "hunter22-secret";
    let typed = session
        .run("type", &json!({"ref": field, "text": secret}))
        .await;
    assert_eq!(typed.typed_secret.as_deref(), Some(secret));
    assert!(typed.text.contains("Typed [secret]"), "{}", typed.text);
    guard.remember(secret);
    let everything =
        |step: &roder_ext_chrome::direct::DirectStep| format!("{} {}", step.text, step.data);
    assert!(!everything(&typed).contains(secret));
    assert!(
        typed.text.contains("(secret field, filled)"),
        "{}",
        typed.text
    );

    let picture = session.run("screenshot", &json!({})).await;
    assert!(!picture.is_error, "{}", picture.text);
    assert_eq!(picture.data["masked"], 1);
    let result = roder_ext_chrome::direct::tool_result("id", "jev_tab_screenshot", &picture);
    assert!(
        result.data[VIEW_IMAGE_DISPLAY_KEY]["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );

    let shown = session
        .run("click", &json!({"ref": look.data["page"]["elements"].as_array().unwrap().iter().find(|e| e["label"] == "Show what I typed").unwrap()["ref"]}))
        .await;
    assert!(shown.text.contains("You typed: [secret]"), "{}", shown.text);
    assert!(!everything(&shown).contains(secret));
    let withheld = session.run("screenshot", &json!({})).await;
    assert!(
        withheld.is_error && withheld.image.is_none(),
        "{}",
        withheld.text
    );
    assert!(withheld.text.contains("withheld"), "{}", withheld.text);
    assert_eq!(guard.scrub(&format!("x {secret}")), "x [secret]");
    session.detach().await;
    drop(page);
}
