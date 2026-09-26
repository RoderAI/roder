//! Observing and acting on one tab.
//!
//! A port of upstream Jev's `browser.py`. The two scripts that must run inside
//! the page — the observation snapshot and the hit-tested act resolver — are
//! vendored verbatim under `assets/`; everything around them is Rust over a
//! direct CDP connection.

use std::time::Duration;

use anyhow::{Context, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::cdp::Connection;
use crate::engine::{JevBrowser, StaleObservation};
use crate::python_json;

const SNAPSHOT_JS: &str = include_str!("assets/snapshot.js");
const ACT_JS: &str = include_str!("assets/act.js");
const SETTLE_JS: &str = include_str!("assets/settle.js");

const VIEWPORT_WIDTH: u32 = 1120;
const VIEWPORT_HEIGHT: u32 = 780;
const LOAD_TIMEOUT: Duration = Duration::from_secs(15);
const OBSERVE_ATTEMPTS: usize = 10;

pub(crate) struct Page {
    connection: Connection,
    target_id: String,
    session: String,
    /// The action whose effects still have to settle before the next read.
    after_input: Option<Value>,
}

impl Page {
    /// Open Jev's own background tab and navigate it.
    pub(crate) async fn open(mut connection: Connection, url: &str) -> anyhow::Result<Self> {
        let target = connection
            .call(
                "Target.createTarget",
                json!({"url": "about:blank", "background": true}),
                None,
            )
            .await?;
        let target_id = target["targetId"]
            .as_str()
            .context("Chrome did not return a target")?
            .to_string();
        let attached = connection
            .call(
                "Target.attachToTarget",
                json!({"targetId": target_id, "flatten": true}),
                None,
            )
            .await?;
        let session = attached["sessionId"]
            .as_str()
            .context("Chrome did not return a session")?
            .to_string();
        let mut page = Self {
            connection,
            target_id,
            session,
            after_input: None,
        };
        page.call(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": VIEWPORT_WIDTH,
                "height": VIEWPORT_HEIGHT,
                "deviceScaleFactor": 1,
                "mobile": false,
            }),
        )
        .await?;
        // Keep rAF and menus rendering in an owned background tab.
        page.call(
            "Emulation.setFocusEmulationEnabled",
            json!({"enabled": true}),
        )
        .await?;
        page.call("Page.navigate", json!({"url": url})).await?;
        page.await_ready().await?;
        Ok(page)
    }

    async fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let session = self.session.clone();
        self.connection.call(method, params, Some(&session)).await
    }

    async fn await_ready(&mut self) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + LOAD_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if let Ok(state) = self.evaluate("document.readyState").await
                && state.as_str() == Some("complete")
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // Upstream proceeds regardless; a slow page is observed as it stands.
        Ok(())
    }

    /// Bring the tab to the front so the run is watchable.
    pub(crate) async fn activate(&mut self) -> anyhow::Result<()> {
        let target_id = self.target_id.clone();
        self.connection
            .call(
                "Target.activateTarget",
                json!({"targetId": target_id}),
                None,
            )
            .await
            .map(|_| ())
    }

    pub(crate) async fn close(&mut self) -> anyhow::Result<()> {
        let target_id = self.target_id.clone();
        self.connection
            .call("Target.closeTarget", json!({"targetId": target_id}), None)
            .await
            .map(|_| ())
    }

    /// `Runtime.evaluate`, mapping a page-side exception to a stale page.
    async fn evaluate(&mut self, expression: &str) -> anyhow::Result<Value> {
        let result = self
            .call(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true}),
            )
            .await?;
        if result.get("exceptionDetails").is_some() {
            return Err(StaleObservation::new("Document changed during evaluation").into());
        }
        Ok(result["result"]["value"].clone())
    }

    async fn evaluate_async(&mut self, expression: &str) -> anyhow::Result<Value> {
        let result = self
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await?;
        if result.get("exceptionDetails").is_some() {
            return Err(StaleObservation::new("Document changed during evaluation").into());
        }
        Ok(result["result"]["value"].clone())
    }

    /// Read the page: settle any pending input, snapshot, then fingerprint.
    pub(crate) async fn observe(&mut self) -> anyhow::Result<Value> {
        if let Some(action) = self.after_input.take() {
            // Read-only, and already logged, so a navigation interrupting it is
            // not an error.
            let _ = self.evaluate_async(&format!("{SETTLE_JS}({action})")).await;
        }
        for attempt in 0..OBSERVE_ATTEMPTS {
            match self.evaluate(SNAPSHOT_JS).await {
                Ok(Value::Null) => {
                    return Err(StaleObservation::new("Document is navigating").into());
                }
                Ok(mut observation) => {
                    let fingerprint = fingerprint(&observation);
                    observation["fingerprint"] = json!(fingerprint);
                    return Ok(observation);
                }
                Err(error) if error.is::<StaleObservation>() && attempt + 1 < OBSERVE_ATTEMPTS => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => return Err(error),
            }
        }
        Err(StaleObservation::new("Page did not settle").into())
    }

    /// Has the page kept the meaning the decision was made against?
    pub(crate) async fn fresh(
        &mut self,
        observation: &Value,
        action: Option<&Value>,
    ) -> anyhow::Result<bool> {
        if let Some(action) = action
            && matches!(action["kind"].as_str(), Some("click") | Some("select"))
        {
            let Some(node) = action["node"].as_i64() else {
                return Ok(false);
            };
            let current = self
                .evaluate(&format!(
                    "(() => {{ const c=window.__jevFast; \
                     return c ? [c.pageKey(),c.guard(c.nodes.get({node}))] : null; }})()"
                ))
                .await?;
            let expected = json!([
                observation["page_key"],
                observation["guards"]
                    .get(node.to_string())
                    .cloned()
                    .unwrap_or(Value::Null),
            ]);
            return Ok(current == expected);
        }
        let marker = self
            .evaluate(&format!(
                "(() => {{ const state={SNAPSHOT_JS}; return state?.marker ?? null; }})()"
            ))
            .await?;
        Ok(marker == observation["marker"])
    }

    /// Execute one observed action.
    pub(crate) async fn act(
        &mut self,
        action: &Value,
        observation: &Value,
        text: Option<&str>,
        wait: Duration,
    ) -> anyhow::Result<()> {
        if !self.fresh(observation, Some(action)).await? {
            return Err(
                StaleObservation::new("Page changed since this decision. Observe again.").into(),
            );
        }
        let kind = action["kind"].as_str().unwrap_or_default();
        match kind {
            "wait" => tokio::time::sleep(wait).await,
            "scroll" => {
                let delta = action["delta"].as_f64().unwrap_or_default();
                self.call(
                    "Input.dispatchMouseEvent",
                    json!({"type": "mouseWheel", "x": 550, "y": 650, "deltaX": 0, "deltaY": delta}),
                )
                .await?;
            }
            _ => self.dispatch_input(action, kind, text).await?,
        }
        self.after_input = (kind != "wait").then(|| action.clone());
        Ok(())
    }

    async fn dispatch_input(
        &mut self,
        action: &Value,
        kind: &str,
        text: Option<&str>,
    ) -> anyhow::Result<()> {
        if !action["node"].is_i64() {
            bail!("Invalid observed node");
        }
        let target = self.evaluate(&format!("{ACT_JS}({action})")).await?;
        if target.is_null() {
            if kind == "select" {
                bail!("Dropdown execution was not confirmed; inspect before retrying.");
            }
            return Err(
                StaleObservation::new("Target changed or is covered. Observe again.").into(),
            );
        }
        if kind == "select" {
            // The script already set the value and fired input/change.
            return Ok(());
        }
        let (x, y) = (target["x"].clone(), target["y"].clone());
        for event in ["mousePressed", "mouseReleased"] {
            self.call(
                "Input.dispatchMouseEvent",
                json!({"type": event, "x": x, "y": y, "button": "left", "clickCount": 1}),
            )
            .await?;
        }
        if kind == "fill" {
            // Select-all through the platform's own accelerator, then replace.
            let modifiers = if cfg!(target_os = "macos") { 4 } else { 2 };
            self.call(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "keyDown", "key": "a", "code": "KeyA",
                    "modifiers": modifiers, "commands": ["selectAll"],
                }),
            )
            .await?;
            self.call(
                "Input.dispatchKeyEvent",
                json!({"type": "keyUp", "key": "a", "code": "KeyA", "modifiers": modifiers}),
            )
            .await?;
            self.call(
                "Input.insertText",
                json!({"text": text.unwrap_or_default()}),
            )
            .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl JevBrowser for Page {
    async fn observe(&mut self) -> anyhow::Result<Value> {
        Page::observe(self).await
    }

    async fn fresh(&mut self, observation: &Value, action: Option<&Value>) -> anyhow::Result<bool> {
        Page::fresh(self, observation, action).await
    }

    async fn act(
        &mut self,
        action: &Value,
        observation: &Value,
        text: Option<&str>,
        wait: Duration,
    ) -> anyhow::Result<()> {
        Page::act(self, action, observation, text, wait).await
    }

    async fn activate(&mut self) -> anyhow::Result<()> {
        Page::activate(self).await
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        Page::close(self).await
    }
}

/// Upstream hashes a canonical dump of exactly these four fields.
pub(crate) fn fingerprint(observation: &Value) -> String {
    let mut content = serde_json::Map::new();
    for key in ["url", "text", "actions", "scroll"] {
        content.insert(
            key.into(),
            observation.get(key).cloned().unwrap_or(Value::Null),
        );
    }
    let canonical = python_json::dumps_sorted(&Value::Object(content));
    Sha256::digest(canonical.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/fingerprint.json")).unwrap()
    }

    #[test]
    fn fingerprint_matches_upstreams_hash_exactly() {
        let fixture = fixture();
        let ours = fingerprint(&fixture["page"]);
        assert_eq!(ours, fixture["upstream"].as_str().unwrap());
        assert_eq!(ours, fixture["sha256"].as_str().unwrap());
    }

    #[test]
    fn fingerprint_canonical_form_matches_upstream() {
        let fixture = fixture();
        let mut content = serde_json::Map::new();
        for key in ["url", "text", "actions", "scroll"] {
            content.insert(key.into(), fixture["page"][key].clone());
        }
        assert_eq!(
            python_json::dumps_sorted(&Value::Object(content)),
            fixture["canonical"].as_str().unwrap()
        );
    }

    #[test]
    fn fingerprint_ignores_fields_upstream_excludes() {
        let fixture = fixture();
        let mut page = fixture["page"].clone();
        let before = fingerprint(&page);
        page["title"] = json!("a different title");
        page["guards"] = json!({});
        page["marker"] = json!([1, 2, 3]);
        assert_eq!(fingerprint(&page), before);

        page["scroll"] = json!({"y": 400, "height": 2400});
        assert_ne!(fingerprint(&page), before);
    }

    #[test]
    fn vendored_scripts_are_the_upstream_ones() {
        // Guards against an edit that would silently change observation.
        assert!(SNAPSHOT_JS.contains("window.__jevFast"));
        assert!(SNAPSHOT_JS.contains("checkVisibility"));
        assert!(ACT_JS.contains("elementFromPoint"));
        assert!(SETTLE_JS.contains("requestAnimationFrame"));
    }
}
