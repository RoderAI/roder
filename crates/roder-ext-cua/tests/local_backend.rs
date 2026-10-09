mod support;
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{ToolContributor, ToolExecutionContext, ToolRegistry};
use roder_ext_cua::{CuaBackend, CuaConfig, CuaToolContributor};
use serde_json::json;
use std::sync::{Arc, atomic::Ordering};
use support::*;

fn local_registry(driver: Arc<Driver>) -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    CuaToolContributor::with_transport(
        CuaConfig {
            backend: CuaBackend::LocalMacos,
            ..Default::default()
        },
        driver,
    )
    .contribute(&mut registry)
    .unwrap();
    registry
}
fn local(thread: &str) -> ToolExecutionContext {
    ToolExecutionContext::new(thread, "turn", PolicyMode::Bypass)
}

#[tokio::test]
async fn local_backend_rejects_remote_context_before_dispatch() {
    let driver = Arc::new(Driver::default());
    let runner = registry(driver.clone());
    assert!(observe(&runner, local("unbound")).await.is_error);
    let registry = local_registry(driver.clone());
    let result = observe(&registry, context("a", "remote")).await;
    assert!(result.is_error);
    assert!(result.text.contains("cannot target the host"));
    assert!(driver.calls.lock().await.is_empty());
}

// Keep the desktop generation assertions in one test: the physical desktop
// deliberately has one process-wide epoch, even across contributor instances.
#[tokio::test]
async fn local_sessions_share_a_desktop_epoch_and_allow_native_unicode_typing() {
    let driver = Arc::new(Driver::default());
    driver.multiple_windows.store(true, Ordering::SeqCst);
    let registry = local_registry(driver.clone());
    let other_registry = local_registry(driver.clone());
    let a = observe(&registry, local("a")).await;
    let b = observe(&other_registry, local("b")).await;
    assert!(!a.is_error && !b.is_error);
    assert_ne!(
        a.data["observation"]["capture_id"],
        b.data["observation"]["capture_id"]
    );
    let typed = call(
        &registry,
        local("a"),
        "cua_type_text",
        json!({"pid":10,"window_id":20,"text":"café 日本語"}),
    )
    .await;
    assert!(!typed.is_error, "{}", typed.text);
    let stale = call(
        &other_registry,
        local("b"),
        "cua_click",
        json!({
            "pid":10,"window_id":20,"x":1,"y":1,
            "capture_id":b.data["observation"]["capture_id"]
        }),
    )
    .await;
    assert!(stale.is_error);
    assert!(stale.text.contains("another local thread"));
    let fresh = observe(&other_registry, local("b")).await;
    let clicked = call(
        &other_registry,
        local("b"),
        "cua_click",
        json!({
            "pid":10,"window_id":20,"x":1,"y":1,
            "capture_id":fresh.data["observation"]["capture_id"]
        }),
    )
    .await;
    assert!(!clicked.is_error);
    driver.scaled_desktop.store(true, Ordering::SeqCst);
    let desktop = call(&registry, local("a"), "cua_get_desktop_state", json!({})).await;
    assert!(!desktop.is_error);
    let moved = call(
        &registry,
        local("a"),
        "cua_move_cursor",
        json!({"x":1,"y":1,"capture_id":desktop.data["observation"]["capture_id"]}),
    )
    .await;
    assert!(!moved.is_error);
    let calls = driver.calls.lock().await;
    assert_eq!(calls.iter().filter(|(name, _)| name == "click").count(), 1);
    assert!(!calls.iter().any(|(name, _)| name == "list_windows"));
    let cursor = calls
        .iter()
        .find(|(name, _)| name == "move_cursor")
        .unwrap();
    assert_eq!(cursor.1["x"], 1.0);
    assert_eq!(cursor.1["y"], 1.0);
    assert!(cursor.1.get("capture_id").is_none());
    assert_eq!(
        calls
            .iter()
            .find(|(name, _)| name == "type_text")
            .unwrap()
            .1["text"],
        "café 日本語"
    );
}
