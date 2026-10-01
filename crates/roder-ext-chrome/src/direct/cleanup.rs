//! Bounded recovery for cancelled or failed input and screenshot commands.
//! The browser may apply a command before its reply reaches us, so arm before
//! sending and disarm only after a successful release reply.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::{DirectTab, TabClient};

#[derive(Clone, Default)]
struct Held {
    mouse: Option<Value>,
    keys: Vec<Value>,
    mask: bool,
}

impl Held {
    fn empty(&self) -> bool {
        self.mouse.is_none() && self.keys.is_empty() && !self.mask
    }
}

#[derive(Clone, Default)]
pub(crate) struct Pending {
    held: Arc<Mutex<Held>>,
    recovery: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<anyhow::Result<()>>>>>,
}

impl Pending {
    pub(crate) fn mask(&self, armed: bool) {
        self.held.lock().unwrap().mask = armed;
    }

    pub(crate) fn before(&self, method: &str, params: &Value) {
        let mut held = self.held.lock().unwrap();
        match (method, params["type"].as_str().unwrap_or_default()) {
            ("Input.dispatchMouseEvent", "mousePressed" | "mouseMoved")
                if params["type"] == "mousePressed" || held.mouse.is_some() =>
            {
                let mut up = params.clone();
                up["type"] = json!("mouseReleased");
                up["buttons"] = json!(0);
                up["modifiers"] = json!(0);
                // Moves during a drag carry the pressed button in our input.
                held.mouse = Some(up);
            }
            ("Input.dispatchKeyEvent", "keyDown" | "rawKeyDown") => {
                let mut up = params.clone();
                let object = up.as_object_mut().unwrap();
                object.remove("text");
                object.remove("unmodifiedText");
                up["type"] = json!("keyUp");
                up["modifiers"] = json!(0);
                held.keys.retain(|key| key["key"] != up["key"]);
                held.keys.push(up);
            }
            _ => {}
        }
    }

    pub(crate) fn after(&self, method: &str, params: &Value) {
        let mut held = self.held.lock().unwrap();
        match (method, params["type"].as_str().unwrap_or_default()) {
            ("Input.dispatchMouseEvent", "mouseReleased") => held.mouse = None,
            ("Input.dispatchKeyEvent", "keyUp") => {
                held.keys.retain(|key| key["key"] != params["key"]);
            }
            _ => {}
        }
    }

    pub(crate) async fn wait(&self, tab: &DirectTab) -> anyhow::Result<()> {
        let mut slot = self.recovery.lock().await;
        if let Some(task) = slot.as_mut() {
            // A failed background attempt leaves the unreleased input armed.
            let _ = task.await;
        }
        *slot = None;
        if !self.held.lock().unwrap().empty() {
            recover(tab.clone(), self.clone()).await?;
        }
        anyhow::ensure!(
            self.held.lock().unwrap().empty(),
            "Browser cleanup incomplete; input refused"
        );
        Ok(())
    }
}

pub(crate) struct Cleanup {
    tab: DirectTab,
    pending: Pending,
}

impl Cleanup {
    pub(crate) fn new(tab: DirectTab, pending: Pending) -> Self {
        Self { tab, pending }
    }

    pub(crate) async fn finish(self) -> anyhow::Result<()> {
        if !self.pending.held.lock().unwrap().empty() {
            recover(self.tab.clone(), self.pending.clone()).await?;
        }
        Ok(())
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.pending.held.lock().unwrap().empty() {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let tab = self.tab.clone();
            let task = runtime.spawn(recover(tab, self.pending.clone()));
            if let Ok(mut slot) = self.pending.recovery.try_lock() {
                *slot = Some(task);
            }
        }
    }
}

fn recover(
    tab: DirectTab,
    pending: Pending,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>> {
    Box::pin(async move {
        let held = pending.held.lock().unwrap().clone();
        let mut client = tokio::time::timeout(Duration::from_secs(2), TabClient::attach(&tab))
            .await
            .map_err(|_| anyhow::anyhow!("Browser cleanup connection timed out"))??;
        let mut failures = Vec::new();
        let mut release = Vec::new();
        if let Some(mouse) = held.mouse {
            release.push(("Input.dispatchMouseEvent", mouse));
        }
        release.extend(
            held.keys
                .into_iter()
                .rev()
                .map(|key| ("Input.dispatchKeyEvent", key)),
        );
        // Each release has its own bound: one failed release cannot skip the others.
        for (method, params) in release {
            match tokio::time::timeout(
                Duration::from_millis(500),
                client.call(method, params.clone()),
            )
            .await
            {
                Ok(Ok(_)) => pending.after(method, &params),
                result => failures.push(format!("{method}: {result:?}")),
            }
        }
        if held.mask {
            match tokio::time::timeout(
                Duration::from_millis(500),
                client.evaluate_isolated("window.__roderDirect?.mask(false)"),
            )
            .await
            {
                Ok(Ok(_)) => pending.mask(false),
                result => failures.push(format!("clear screenshot mask: {result:?}")),
            }
        }
        anyhow::ensure!(
            failures.is_empty(),
            "Browser cleanup failed; input remains blocked: {}",
            failures.join("; ")
        );
        Ok(())
    })
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
