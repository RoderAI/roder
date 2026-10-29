//! Publish an actionable parent observation and seed the direct-tools baseline.
use super::SessionState;
use crate::{fallback::guard::JevGuard, runner::JevRequest};
use roder_ext_chrome::{
    direct::{DirectSession, DirectTab},
    semantic,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub(super) async fn publish(
    data: &mut Value,
    state: &SessionState,
    request: &JevRequest,
    deadline: tokio::time::Instant,
) {
    if matches!(
        data["status"].as_str(),
        Some("access_denied" | "needs_confirmation" | "closed")
    ) {
        return;
    }
    let (Some(endpoint), Some(tab)) = (&state.endpoint, state.tabs.current()) else {
        return;
    };
    let guard = Arc::new(JevGuard::new(
        request.scope.clone(),
        request.confirm_irreversible,
        request.refuse_cookie_banners,
        state.secrets.clone(),
    ));
    let target = DirectTab::Target {
        endpoint: endpoint.url().to_string(),
        target_id: tab.target.clone(),
    };
    let read = async {
        let mut session = DirectSession::attach(&target, guard, false).await.ok()?;
        let step = session.run("look", &json!({})).await;
        (!step.is_error).then_some(step.data["page"].clone())
    };
    let until = deadline.min(tokio::time::Instant::now() + Duration::from_secs(2));
    if let Ok(Some(page)) = tokio::time::timeout_at(until, read).await {
        data["page"]["snapshot"] = page["snapshot"].clone();
        data["page"]["dom_snapshot"] = json!(semantic::render(&page["snapshot"], 100));
        data["page"]["reference_tools"] = json!("jev_tab_*");
        data["page"]["elements"] = page["elements"].clone();
        data["page"]["viewport"] = page["viewport"].clone();
    }
}
