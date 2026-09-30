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
    recovery: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
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

    fn take(&self) -> Held {
        std::mem::take(&mut *self.held.lock().unwrap())
    }

    pub(crate) async fn wait(&self) {
        let mut slot = self.recovery.lock().await;
        if let Some(task) = slot.as_mut() {
            let _ = task.await;
        }
        *slot = None;
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

    pub(crate) async fn finish(self) {
        let held = self.pending.held.lock().unwrap().clone();
        if !held.empty() {
            recover(self.tab.clone(), held).await;
        }
        self.pending.take();
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let held = self.pending.take();
        if held.empty() {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let tab = self.tab.clone();
            let task = runtime.spawn(recover(tab, held));
            if let Ok(mut slot) = self.pending.recovery.try_lock() {
                *slot = Some(task);
            }
        }
    }
}

fn recover(
    tab: DirectTab,
    held: Held,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        // Reconnect: a cancelled command may have left the old socket unusable.
        // Never wait a full command timeout while shutting a task down.
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            let mut client = TabClient::attach(&tab).await?;
            if let Some(mouse) = held.mouse {
                client.call("Input.dispatchMouseEvent", mouse).await?;
            }
            for key in held.keys.into_iter().rev() {
                client.call("Input.dispatchKeyEvent", key).await?;
            }
            if held.mask {
                client
                    .evaluate_isolated("window.__roderDirect?.mask(false)")
                    .await?;
            }
            anyhow::Ok(())
        })
        .await;
    })
}
