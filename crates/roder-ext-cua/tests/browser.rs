#[allow(dead_code)]
mod support;
use async_trait::async_trait;
use roder_api::{
    policy_mode::PolicyMode,
    tools::{ToolContributor, ToolRegistry},
};
use roder_ext_cua::{CuaConfig, CuaTarget, CuaToolContributor, CuaTransport, DriverReply};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use support::{call, context, observe};
use tokio::sync::Mutex;

#[derive(Default)]
struct Browser {
    calls: Mutex<Vec<(String, Value)>>,
    native: support::Driver,
    refuse: AtomicBool,
    pending: AtomicBool,
    corrupt: AtomicBool,
}
#[async_trait]
impl CuaTransport for Browser {
    async fn call(
        &self,
        target: CuaTarget<'_>,
        name: &str,
        args: Value,
        timeout: u64,
    ) -> anyhow::Result<DriverReply> {
        self.calls.lock().await.push((name.into(), args.clone()));
        let observation = match name {
            "get_browser_state" if args.get("target_id").is_none() => {
                json!({"status":"ok","target_id":"target-one","binding_quality":"exact","tabs":[{"tab_id":"tab-one","active":true}]})
            }
            "get_browser_state" => {
                let n = self.calls.lock().await.len();
                json!({"status":"ok","target_id":"target-one","tab_id":"tab-one","refs":[{"ref":format!("p{n}:1"),"actions":["click","type"]}],"screenshot_width":2,"screenshot_height":2,
                    "screenshot_png_b64":if self.corrupt.load(Ordering::SeqCst) {"invalid"} else {include_str!("capture.b64")}})
            }
            "browser_click" | "browser_type" | "browser_navigate" => {
                if self.pending.load(Ordering::SeqCst) {
                    std::future::pending::<()>().await;
                }
                if self.refuse.load(Ordering::SeqCst) {
                    return Ok(DriverReply {
                        observation: json!({"status":"refused","refusal":{"code":"browser_input_trust_unavailable"}}),
                        is_error: true,
                    });
                }
                json!({"status":"ok"})
            }
            "browser_prepare" | "start_session" => json!({"status":"ok"}),
            _ => return self.native.call(target, name, args, timeout).await,
        };
        Ok(DriverReply {
            observation,
            is_error: false,
        })
    }
}
fn registry(driver: Arc<Browser>, grant: bool) -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    CuaToolContributor::with_transport(
        CuaConfig {
            allow_existing_browser_profile: grant,
            ..Default::default()
        },
        driver,
    )
    .contribute(&mut registry)
    .unwrap();
    registry
}
async fn bind(registry: &ToolRegistry) {
    let r = call(
        registry,
        context("a", "one"),
        "cua_get_browser_state",
        json!({"pid":10,"window_id":20}),
    )
    .await;
    assert!(!r.is_error, "{}", r.text);
}
async fn snapshot(registry: &ToolRegistry) -> Value {
    let r = call(
        registry,
        context("a", "one"),
        "cua_get_browser_state",
        json!({"target_id":"target-one","tab_id":"tab-one"}),
    )
    .await;
    assert!(!r.is_error, "{}", r.text);
    assert!(r.data.get("__view_image").is_some());
    assert!(r.data["observation"].get("screenshot_png_b64").is_none());
    r.data["observation"]["refs"][0]["ref"].clone()
}
#[tokio::test]
async fn foreign_targets_and_stale_refs_never_dispatch_browser_input() {
    let driver = Arc::new(Browser::default());
    let registry = registry(driver.clone(), false);
    bind(&registry).await;
    let old = snapshot(&registry).await;
    let fresh = snapshot(&registry).await;
    for (ctx, reference) in [
        (context("b", "one"), fresh.clone()),
        (context("a", "two"), fresh.clone()),
        (context("a", "one"), old),
    ] {
        assert!(
            call(
                &registry,
                ctx,
                "cua_browser_click",
                json!({"target_id":"target-one","tab_id":"tab-one","ref":reference})
            )
            .await
            .is_error
        );
    }
    assert_eq!(driver.calls.lock().await.len(), 4);
    assert!(!call(&registry,context("a","one"),"cua_browser_type",json!({"target_id":"target-one","tab_id":"tab-one","ref":fresh,"text":"café 日本語","replace":true})).await.is_error);
}
#[tokio::test]
async fn native_actions_revoke_browser_refs_and_plan_never_prepares_or_navigates() {
    let driver = Arc::new(Browser::default());
    let registry = registry(driver.clone(), true);
    bind(&registry).await;
    let reference = snapshot(&registry).await;
    observe(&registry, context("a", "one")).await;
    assert!(
        !call(
            &registry,
            context("a", "one"),
            "cua_press_key",
            json!({"pid":10,"window_id":20,"key":"Enter"})
        )
        .await
        .is_error
    );
    assert!(
        call(
            &registry,
            context("a", "one"),
            "cua_browser_click",
            json!({"target_id":"target-one","tab_id":"tab-one","ref":reference})
        )
        .await
        .is_error
    );
    let before = driver.calls.lock().await.len();
    let mut ctx = context("a", "one");
    ctx.effective_mode = PolicyMode::Plan;
    for (name, args) in [
        (
            "cua_browser_navigate",
            json!({"target_id":"target-one","tab_id":"tab-one","url":"https://example.com"}),
        ),
        (
            "cua_browser_prepare",
            json!({"pid":10,"window_id":20,"profile_mode":"existing_profile"}),
        ),
    ] {
        assert!(call(&registry, ctx.clone(), name, args).await.is_error);
    }
    assert_eq!(driver.calls.lock().await.len(), before);
}
#[tokio::test]
async fn existing_profile_requires_config_grant_and_observed_window_and_revokes_bindings() {
    let driver = Arc::new(Browser::default());
    let denied = registry(driver.clone(), false);
    observe(&denied, context("a", "one")).await;
    assert!(
        call(
            &denied,
            context("a", "one"),
            "cua_browser_prepare",
            json!({"pid":10,"window_id":20,"profile_mode":"existing_profile"})
        )
        .await
        .is_error
    );
    assert_eq!(driver.calls.lock().await.len(), 1);
    let allowed = registry(driver.clone(), true);
    bind(&allowed).await;
    assert!(
        call(
            &allowed,
            context("a", "one"),
            "cua_browser_prepare",
            json!({"pid":10,"window_id":20,"profile_mode":"existing_profile"})
        )
        .await
        .is_error
    );
    observe(&allowed, context("a", "one")).await;
    assert!(
        !call(
            &allowed,
            context("a", "one"),
            "cua_browser_prepare",
            json!({"pid":10,"window_id":20,"profile_mode":"existing_profile","profile_name":null})
        )
        .await
        .is_error
    );
    assert_eq!(
        driver.calls.lock().await.last().unwrap().1["strategy"],
        json!({"kind":"existing_profile"})
    );
    assert!(
        call(
            &allowed,
            context("a", "one"),
            "cua_get_browser_state",
            json!({"target_id":"target-one","tab_id":"tab-one"})
        )
        .await
        .is_error
    );
}
#[tokio::test]
async fn refused_input_has_fresh_image_without_fallback_and_bad_capture_revokes_refs() {
    let driver = Arc::new(Browser::default());
    let registry = registry(driver.clone(), false);
    bind(&registry).await;
    let reference = snapshot(&registry).await;
    driver.refuse.store(true, Ordering::SeqCst);
    let r=call(&registry,context("a","one"),"cua_browser_click",json!({"target_id":"target-one","tab_id":"tab-one","ref":reference,"delivery_mode":"background"})).await;
    assert!(r.is_error);
    assert!(r.data.get("__view_image").is_some());
    assert!(r.text.contains("browser_input_trust_unavailable"));
    assert_eq!(
        driver
            .calls
            .lock()
            .await
            .iter()
            .filter(|(n, _)| n == "browser_click")
            .count(),
        1
    );
    driver.corrupt.store(true, Ordering::SeqCst);
    assert!(
        call(
            &registry,
            context("a", "one"),
            "cua_get_browser_state",
            json!({"target_id":"target-one","tab_id":"tab-one"})
        )
        .await
        .is_error
    );
    assert!(
        call(
            &registry,
            context("a", "one"),
            "cua_browser_click",
            json!({"target_id":"target-one","tab_id":"tab-one","ref":reference})
        )
        .await
        .is_error
    );
}
#[tokio::test]
async fn cancelled_browser_action_revokes_refs_before_dispatch() {
    let driver = Arc::new(Browser::default());
    let registry = Arc::new(registry(driver.clone(), false));
    bind(&registry).await;
    let reference = snapshot(&registry).await;
    driver.pending.store(true, Ordering::SeqCst);
    let r = registry.clone();
    let token = reference.clone();
    let task = tokio::spawn(async move {
        call(
            &r,
            context("a", "one"),
            "cua_browser_click",
            json!({"target_id":"target-one","tab_id":"tab-one","ref":token}),
        )
        .await
    });
    while driver.calls.lock().await.len() < 4 {
        tokio::task::yield_now().await;
    }
    task.abort();
    let _ = task.await;
    driver.pending.store(false, Ordering::SeqCst);
    assert!(
        call(
            &registry,
            context("a", "one"),
            "cua_browser_click",
            json!({"target_id":"target-one","tab_id":"tab-one","ref":reference})
        )
        .await
        .is_error
    );
    assert_eq!(driver.calls.lock().await.len(), 4);
}

#[tokio::test]
async fn ending_browser_session_revokes_targets_before_driver_cleanup() {
    let driver = Arc::new(Browser::default());
    let registry = registry(driver.clone(), false);
    bind(&registry).await;
    snapshot(&registry).await;
    assert!(
        !call(
            &registry,
            context("a", "one"),
            "cua_end_browser_session",
            json!({})
        )
        .await
        .is_error
    );
    assert_eq!(driver.calls.lock().await.last().unwrap().0, "end_session");
    assert!(
        call(
            &registry,
            context("a", "one"),
            "cua_get_browser_state",
            json!({"target_id":"target-one","tab_id":"tab-one"})
        )
        .await
        .is_error
    );
}
