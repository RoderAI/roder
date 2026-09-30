use super::ChromeBridge;
use serde_json::{Value, json};
use tokio::sync::mpsc;

pub(super) struct Pending<'a> {
    pub(super) bridge: &'a ChromeBridge,
    pub(super) corr: &'a str,
    pub(super) sender: mpsc::UnboundedSender<Value>,
}
impl Drop for Pending<'_> {
    fn drop(&mut self) {
        let unanswered = self.bridge.lock().pending.remove(self.corr).is_some();
        if unanswered {
            let _ = self
                .sender
                .send(json!({"type":"command/cancel", "targetId":self.corr}));
        }
    }
}
