//! The fetch and XHR requests each session's page has open.
//!
//! A page that answers an input with a request (a cart that adds over fetch,
//! then redraws) leaves its document as it was until the response arrives, so
//! a quiet, unchanged page with one open has not yet reacted. The ledger
//! counts the requests the Network domain reports, so the settle can ask
//! whether one started after the input is still waiting for its answer. Only
//! the page's own calls count: a document, image or script request is the
//! load the ready check already waits for. The rule follows fastbrowse's
//! browser session (MIT).

use std::collections::HashMap;

use serde_json::Value;

#[derive(Default)]
pub(super) struct Requests {
    /// Fetch and XHR requests started, counted; a mark is a place in it.
    started: u64,
    /// (session, request id) -> its place in that count, while unanswered.
    open: HashMap<(String, String), u64>,
    /// The count when the first input since the last [`Requests::take_mark`]
    /// was sent.
    mark: Option<u64>,
}

impl Requests {
    /// Keep the ledger from one DevTools event.
    pub(super) fn on_event(&mut self, event: &Value) {
        let params = &event["params"];
        let session = event["sessionId"].as_str();
        match (event["method"].as_str(), session) {
            (Some("Network.requestWillBeSent"), Some(session))
                if matches!(params["type"].as_str(), Some("Fetch" | "XHR")) =>
            {
                let key = (session.to_string(), request_id(params));
                // A redirect is sent again under the same id and keeps the
                // place it started with.
                if !self.open.contains_key(&key) {
                    self.started += 1;
                    self.open.insert(key, self.started);
                }
            }
            (Some("Network.loadingFinished" | "Network.loadingFailed"), Some(session)) => {
                self.open.remove(&(session.to_string(), request_id(params)));
            }
            // A tab that went away answers nothing more.
            (Some("Target.detachedFromTarget"), _) => {
                let gone = params["sessionId"].as_str().unwrap_or_default();
                self.open.retain(|(session, _), _| session != gone);
            }
            _ => {}
        }
    }

    /// An input is being sent: the first since the last mark was taken sets
    /// it, so the requests the whole action started count.
    pub(super) fn input_sent(&mut self) {
        self.mark.get_or_insert(self.started);
    }

    /// The mark the inputs since the last call set, if any, cleared.
    pub(super) fn take_mark(&mut self) -> Option<u64> {
        self.mark.take()
    }

    /// Whether a request `session`'s page started after `mark` is still
    /// waiting for its response.
    pub(super) fn open_since(&self, session: &str, mark: u64) -> bool {
        self.open
            .iter()
            .any(|((owner, _), started)| owner == session && *started > mark)
    }
}

fn request_id(params: &Value) -> String {
    params["requestId"].as_str().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn event(method: &str, session: &str, params: Value) -> Value {
        json!({"method": method, "sessionId": session, "params": params})
    }

    fn sent(id: &str, kind: &str) -> Value {
        event(
            "Network.requestWillBeSent",
            "tab",
            json!({"requestId": id, "type": kind}),
        )
    }

    #[test]
    fn only_requests_started_after_the_mark_and_still_open_count() {
        let mut requests = Requests::default();
        requests.on_event(&sent("poll", "Fetch"));
        requests.input_sent();
        requests.on_event(&sent("early", "XHR"));
        // A second input of the same action keeps the first one's mark.
        requests.input_sent();
        let mark = requests.take_mark().unwrap();
        assert_eq!(requests.take_mark(), None);
        assert!(requests.open_since("tab", mark));
        assert!(!requests.open_since("other", mark));
        requests.on_event(&event(
            "Network.loadingFinished",
            "tab",
            json!({"requestId": "early"}),
        ));
        // The poll started before the input and is not the action's.
        assert!(!requests.open_since("tab", mark));
    }

    #[test]
    fn loads_redirects_and_closed_tabs_hold_nothing_open() {
        let mut requests = Requests::default();
        requests.input_sent();
        let mark = requests.take_mark().unwrap();
        requests.on_event(&sent("image", "Image"));
        requests.on_event(&sent("document", "Document"));
        assert!(!requests.open_since("tab", mark));
        requests.on_event(&sent("api", "Fetch"));
        requests.on_event(&sent("api", "Fetch"));
        requests.on_event(&event(
            "Network.loadingFailed",
            "tab",
            json!({"requestId": "api"}),
        ));
        assert!(!requests.open_since("tab", mark));
        requests.on_event(&sent("hung", "XHR"));
        requests.on_event(&json!({
            "method": "Target.detachedFromTarget",
            "params": {"sessionId": "tab"},
        }));
        assert!(!requests.open_since("tab", mark));
    }
}
