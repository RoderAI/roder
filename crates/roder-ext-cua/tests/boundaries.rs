mod support;

use roder_api::policy_mode::PolicyMode;
use roder_api::remote_runner::RemoteWorkspace;
use roder_api::tools::{ToolCall, ToolExecutionContext};
use roder_ext_cua::{CuaConfig, CuaTransport, RunnerCuaTransport};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::Ordering};
use support::*;

#[tokio::test]
async fn fails_closed_without_runner_or_with_model_owned_routing() {
    let driver = Arc::new(Driver::default());
    let registry = registry(driver.clone());
    assert!(
        call(
            &registry,
            ToolExecutionContext::new("a", "turn", PolicyMode::Bypass),
            "cua_list_windows",
            json!({})
        )
        .await
        .is_error
    );
    for args in [
        json!({"session":"other"}),
        json!({"sandbox":"other"}),
        json!({"pid":true}),
    ] {
        assert!(
            call(&registry, context("a", "one"), "cua_list_windows", args)
                .await
                .is_error
        );
    }
    assert!(driver.calls.lock().await.is_empty());
}
#[tokio::test]
async fn pixel_input_requires_owned_current_png_and_returns_new_image_on_refusal() {
    let driver = Arc::new(Driver::default());
    let registry = registry(driver.clone());
    let ctx = context("a", "one");
    let read = observe(&registry, ctx.clone()).await;
    assert!(!read.is_error, "{}", read.text);
    let capture = read.data["observation"]["capture_id"].clone();
    for (ctx, args) in [
        (
            context("b", "one"),
            json!({"pid":10,"window_id":20,"x":1,"y":1,"capture_id":capture}),
        ),
        (
            context("a", "two"),
            json!({"pid":10,"window_id":20,"x":1,"y":1,"capture_id":capture}),
        ),
        (
            ctx.clone(),
            json!({"pid":10,"window_id":21,"x":1,"y":1,"capture_id":capture}),
        ),
        (
            ctx.clone(),
            json!({"pid":10,"window_id":20,"x":2,"y":1,"capture_id":capture}),
        ),
    ] {
        assert!(call(&registry, ctx, "cua_click", args).await.is_error);
    }
    driver.fail_input.store(true, Ordering::SeqCst);
    let result = call(
        &registry,
        ctx.clone(),
        "cua_click",
        json!({"pid":10,"window_id":20,"x":1,"y":1,"capture_id":capture}),
    )
    .await;
    assert!(result.is_error);
    assert!(result.data.get("__view_image").is_some());
    assert!(
        result.data["after_action"]
            .get("screenshot_png_b64")
            .is_none()
    );
    assert_eq!(
        driver
            .calls
            .lock()
            .await
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["get_window_state", "click", "get_window_state"]
    );
    assert!(
        call(
            &registry,
            ctx,
            "cua_click",
            json!({"pid":10,"window_id":20,"x":1,"y":1,"capture_id":capture})
        )
        .await
        .is_error
    );
}
#[tokio::test]
async fn failed_capture_revokes_grounding_and_plan_input_never_reaches_driver() {
    let driver = Arc::new(Driver::default());
    let registry = registry(driver.clone());
    let ctx = context("a", "one");
    assert!(!observe(&registry, ctx.clone()).await.is_error);
    let mut plan = ctx.clone();
    plan.effective_mode = PolicyMode::Plan;
    assert!(
        call(
            &registry,
            plan,
            "cua_press_key",
            json!({"pid":10,"window_id":20,"key":"7"})
        )
        .await
        .is_error
    );
    driver.fail_capture.store(true, Ordering::SeqCst);
    assert!(observe(&registry, ctx.clone()).await.is_error);
    assert!(
        call(
            &registry,
            ctx,
            "cua_press_key",
            json!({"pid":10,"window_id":20,"key":"7"})
        )
        .await
        .is_error
    );
    assert_eq!(driver.calls.lock().await.len(), 2);
}
#[tokio::test]
async fn runner_transport_reuses_context_and_cancels_a_dropped_command() {
    let config = CuaConfig::default();
    let transport = Arc::new(RunnerCuaTransport::new(&config));
    let runner = Arc::new(Runner::new("one"));
    let workspace = RemoteWorkspace {
        session: runner.clone(),
        root: "/workspace".into(),
        read_roots: vec![],
    };
    transport
        .call(
            &workspace,
            "press_key",
            json!({"key":"$HOME; $(echo nope)","session":"roder-safe"}),
            1000,
        )
        .await
        .unwrap();
    let requests = runner.requests.lock().await;
    assert_eq!(requests[0].program, config.program);
    assert!(requests[0].env.is_empty());
    assert_eq!(requests[0].timeout_ms, Some(1000));
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].args[1]).unwrap()["key"],
        "$HOME; $(echo nope)"
    );
    drop(requests);
    let mut pending = Runner::new("pending");
    pending.pending = true;
    let pending = Arc::new(pending);
    let ws = RemoteWorkspace {
        session: pending.clone(),
        root: "/workspace".into(),
        read_roots: vec![],
    };
    let task =
        tokio::spawn(async move { transport.call(&ws, "press_key", json!({}), 10_000).await });
    while pending.requests.lock().await.is_empty() {
        tokio::task::yield_now().await;
    }
    task.abort();
    let _ = task.await;
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !pending.cancelled.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn provider_nulls_are_omissions_but_unknown_null_routing_still_fails() {
    let driver = Arc::new(Driver::default());
    let registry = registry(driver.clone());
    assert!(
        !call(
            &registry,
            context("a", "one"),
            "cua_get_window_state",
            json!({"pid":10,"window_id":20,"max_depth":null,"query":null})
        )
        .await
        .is_error
    );
    assert!(
        call(
            &registry,
            context("a", "one"),
            "cua_list_windows",
            json!({"sandbox":null})
        )
        .await
        .is_error
    );
    assert!(
        call(
            &registry,
            context("a", "one"),
            "cua_get_window_state",
            json!({"pid":null,"window_id":20})
        )
        .await
        .is_error
    );
    assert_eq!(driver.calls.lock().await.len(), 1);
}

#[tokio::test]
async fn strict_provider_schema_can_express_pixel_or_element_input() {
    let driver = Arc::new(Driver::default());
    let registry = registry(driver.clone());
    let ctx = context("a", "one");
    let schema = registry.get("cua_click").unwrap().spec().parameters;
    assert_eq!(
        schema["properties"]["element_token"]["type"],
        json!(["string", "null"])
    );
    assert_eq!(
        schema["properties"]["button"]["enum"],
        json!(["left", "right", "middle", null])
    );
    let read = observe(&registry, ctx.clone()).await;
    let result=call(&registry,ctx,"cua_click",json!({"pid":10,"window_id":20,"x":1,"y":1,
        "capture_id":read.data["observation"]["capture_id"],"element_token":null,"button":null,"count":null,
        "modifier":null,"delivery_mode":null})).await;
    assert!(!result.is_error, "{}", result.text);
    assert!(
        driver.calls.lock().await[1]
            .1
            .get("element_token")
            .is_none()
    );
}

#[tokio::test]
async fn cancelling_input_revokes_grounding_until_another_observation() {
    let driver = Arc::new(Driver::default());
    let registry = registry(driver.clone());
    let ctx = context("a", "one");
    let read = observe(&registry, ctx.clone()).await;
    let capture = read.data["observation"]["capture_id"].clone();
    driver.pending_input.store(true, Ordering::SeqCst);
    let executor = registry.get("cua_click").unwrap();
    let task_ctx = ctx.clone();
    let task_capture = capture.clone();
    let task = tokio::spawn(async move {
        executor.execute(task_ctx.clone(),ToolCall { id:"pending".into(),name:"cua_click".into(),
        arguments:json!({"pid":10,"window_id":20,"x":1,"y":1,"capture_id":task_capture}),raw_arguments:String::new(),
        thread_id:task_ctx.thread_id,turn_id:task_ctx.turn_id }).await
    });
    while driver.calls.lock().await.len() < 2 {
        tokio::task::yield_now().await;
    }
    task.abort();
    let _ = task.await;
    driver.pending_input.store(false, Ordering::SeqCst);
    assert!(
        call(
            &registry,
            ctx.clone(),
            "cua_click",
            json!({"pid":10,"window_id":20,"x":1,"y":1,"capture_id":capture})
        )
        .await
        .is_error
    );
    assert_eq!(driver.calls.lock().await.len(), 2);
    assert!(!observe(&registry, ctx.clone()).await.is_error);
    assert!(
        !call(
            &registry,
            ctx,
            "cua_press_key",
            json!({"pid":10,"window_id":20,"key":"END"})
        )
        .await
        .is_error
    );
}

#[tokio::test]
async fn unicode_uses_exact_value_and_unsafe_typing_is_refused_before_input() {
    let driver = Arc::new(Driver::default());
    let registry = registry(driver.clone());
    let ctx = context("a", "one");
    let read = observe(&registry, ctx.clone()).await;
    let token = format!(
        "{}:0",
        read.data["observation"]["snapshot_id"].as_str().unwrap()
    );
    assert!(
        call(
            &registry,
            ctx.clone(),
            "cua_type_text",
            json!({"pid":10,"window_id":20,"text":"日本語"})
        )
        .await
        .is_error
    );
    assert_eq!(driver.calls.lock().await.len(), 1);
    assert!(call(&registry,ctx.clone(),"cua_set_value",json!({"pid":10,"window_id":20,"value":"raw token forbidden","element_token":"s00000001:0"})).await.is_error);
    assert_eq!(driver.calls.lock().await.len(), 1);
    assert!(
        !call(
            &registry,
            ctx.clone(),
            "cua_set_value",
            json!({"pid":10,"window_id":20,"value":"café λ 日本語","element_token":token})
        )
        .await
        .is_error
    );
    assert_eq!(driver.calls.lock().await[1].1["value"], "café λ 日本語");
    assert_eq!(
        driver.calls.lock().await[1].1["element_token"],
        "s00000001:0"
    );
    driver.multiple_windows.store(true, Ordering::SeqCst);
    assert!(
        call(
            &registry,
            ctx.clone(),
            "cua_type_text",
            json!({"pid":10,"window_id":20,"text":"dialog"})
        )
        .await
        .is_error
    );
    assert_eq!(
        driver
            .calls
            .lock()
            .await
            .iter()
            .filter(|(name, _)| name == "type_text")
            .count(),
        0
    );
    assert!(
        call(
            &registry,
            ctx,
            "cua_press_key",
            json!({"pid":10,"window_id":20,"key":"7"})
        )
        .await
        .is_error
    );
}
