//! What the browser reports of documents and downloads, from the events the
//! connection reads anyway.
//!
//! - The HTTP status of each tab's committed document: the status of the
//!   `Document` response whose loader the main frame then navigated to. A
//!   response that never committed (a link to a file, which Chrome turns
//!   into a download and the tab keeps its page) is not the page's status,
//!   and a page script cannot change it the way it can shadow
//!   `performance`.
//! - Document requests an input started: a link whose target is a file
//!   begins as a navigation, and the download it becomes is announced a
//!   moment after the page has settled unchanged.
//! - Downloads, from the `Browser.download*` events a browser sends once
//!   `Browser.setDownloadBehavior` enabled them.
//!
//! The download rule follows fastbrowse's browser session (MIT), which reads
//! the file off a paused response instead; Jev reads no paused requests, so
//! it lets Chrome save the file and reads it from disk.

use std::collections::{HashMap, VecDeque};

use serde_json::Value;

/// How many document responses are kept for a commit to find its status.
const RESPONSES_KEPT: usize = 64;

/// One download the browser announced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DownloadEvent {
    pub(crate) guid: String,
    pub(crate) url: String,
    /// The name the response suggests, as Chrome derived it.
    pub(crate) name: String,
    /// The frame whose navigation or script started it; a tab's main frame
    /// has the tab's target id.
    pub(crate) frame: String,
    pub(crate) state: DownloadState,
    pub(crate) bytes: u64,
    /// Where Chrome saved it, once complete, when it says.
    pub(crate) file_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DownloadState {
    InProgress,
    Completed,
    Canceled,
}

/// A place in the ledger: the counts when the first input of an action was
/// sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NavigationMark {
    documents: u64,
    downloads: usize,
}

#[derive(Default)]
pub(super) struct Navigation {
    /// Document requests started, counted; a mark is a place in it.
    documents: u64,
    /// (session, request id) -> its place in that count, while unanswered.
    open: HashMap<(String, String), u64>,
    /// Loader id -> the status its document was served with, newest last.
    responses: VecDeque<(String, u16)>,
    /// Session -> the loader its main frame committed last.
    committed: HashMap<String, String>,
    downloads: Vec<DownloadEvent>,
    mark: Option<NavigationMark>,
}

impl Navigation {
    /// Keep the ledger from one DevTools event.
    pub(super) fn on_event(&mut self, event: &Value) {
        let params = &event["params"];
        let session = event["sessionId"].as_str();
        let text = |key: &str| params[key].as_str().unwrap_or_default().to_string();
        match (event["method"].as_str(), session) {
            (Some("Network.requestWillBeSent"), Some(session)) if params["type"] == "Document" => {
                let key = (session.to_string(), text("requestId"));
                if !self.open.contains_key(&key) {
                    self.documents += 1;
                    self.open.insert(key, self.documents);
                }
            }
            (Some("Network.responseReceived"), Some(_)) if params["type"] == "Document" => {
                let status = params["response"]["status"].as_u64().unwrap_or_default();
                if let (Ok(status), Some(loader)) =
                    (u16::try_from(status), params["loaderId"].as_str())
                    && status > 0
                {
                    self.responses.retain(|(kept, _)| kept != loader);
                    self.responses.push_back((loader.to_string(), status));
                    while self.responses.len() > RESPONSES_KEPT {
                        self.responses.pop_front();
                    }
                }
            }
            (Some("Network.loadingFinished" | "Network.loadingFailed"), Some(session)) => {
                self.open.remove(&(session.to_string(), text("requestId")));
            }
            (Some("Page.frameNavigated"), Some(session))
                if params["frame"]["parentId"].is_null() =>
            {
                if let Some(loader) = params["frame"]["loaderId"].as_str() {
                    self.committed
                        .insert(session.to_string(), loader.to_string());
                }
            }
            (Some("Target.detachedFromTarget"), _) => {
                let gone = params["sessionId"].as_str().unwrap_or_default();
                self.open.retain(|(session, _), _| session != gone);
                self.committed.remove(gone);
            }
            (Some("Browser.downloadWillBegin"), _) => {
                let guid = text("guid");
                if !self.downloads.iter().any(|download| download.guid == guid) {
                    self.downloads.push(DownloadEvent {
                        guid,
                        url: text("url"),
                        name: text("suggestedFilename"),
                        frame: text("frameId"),
                        state: DownloadState::InProgress,
                        bytes: 0,
                        file_path: None,
                    });
                }
            }
            (Some("Browser.downloadProgress"), _) => {
                let guid = text("guid");
                let Some(download) = self
                    .downloads
                    .iter_mut()
                    .find(|download| download.guid == guid)
                else {
                    return;
                };
                download.state = match params["state"].as_str() {
                    Some("completed") => DownloadState::Completed,
                    Some("canceled") => DownloadState::Canceled,
                    _ => DownloadState::InProgress,
                };
                let received = params["receivedBytes"].as_u64().unwrap_or_default();
                download.bytes = download.bytes.max(received);
                if let Some(path) = params["filePath"].as_str().filter(|path| !path.is_empty()) {
                    download.file_path = Some(path.to_string());
                }
            }
            _ => {}
        }
    }

    /// An input is being sent: the first since the last mark was taken sets
    /// it, so what the whole action started counts.
    pub(super) fn input_sent(&mut self) {
        self.mark.get_or_insert(NavigationMark {
            documents: self.documents,
            downloads: self.downloads.len(),
        });
    }

    /// The mark the inputs since the last call set, if any, cleared.
    pub(super) fn take_mark(&mut self) -> Option<NavigationMark> {
        self.mark.take()
    }

    /// The downloads announced since `mark`, oldest first.
    pub(super) fn downloads_since(&self, mark: NavigationMark) -> &[DownloadEvent] {
        self.downloads.get(mark.downloads..).unwrap_or_default()
    }

    /// Whether a document request `session`'s page started after `mark` is
    /// still waiting to resolve.
    pub(super) fn documents_open_since(&self, session: &str, mark: NavigationMark) -> bool {
        self.open
            .iter()
            .any(|((owner, _), started)| owner == session && *started > mark.documents)
    }

    /// The HTTP status `session`'s committed document was served with, when
    /// the browser reported both.
    pub(super) fn document_status(&self, session: &str) -> Option<u16> {
        let loader = self.committed.get(session)?;
        self.responses
            .iter()
            .rev()
            .find(|(kept, _)| kept == loader)
            .map(|(_, status)| *status)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn event(method: &str, session: Option<&str>, params: Value) -> Value {
        let mut event = json!({"method": method, "params": params});
        if let Some(session) = session {
            event["sessionId"] = json!(session);
        }
        event
    }

    fn response(loader: &str, status: u16) -> Value {
        event(
            "Network.responseReceived",
            Some("tab"),
            json!({"type": "Document", "loaderId": loader, "response": {"status": status}}),
        )
    }

    fn committed(loader: &str) -> Value {
        event(
            "Page.frameNavigated",
            Some("tab"),
            json!({"frame": {"id": "main", "loaderId": loader}}),
        )
    }

    #[test]
    fn the_status_is_the_committed_documents_not_the_last_response() {
        let mut ledger = Navigation::default();
        assert_eq!(ledger.document_status("tab"), None);
        ledger.on_event(&response("first", 503));
        ledger.on_event(&committed("first"));
        assert_eq!(ledger.document_status("tab"), Some(503));
        // A file the page linked to answered 200 and never committed.
        ledger.on_event(&response("file", 200));
        assert_eq!(ledger.document_status("tab"), Some(503));
        // A frame inside the page commits without moving the page.
        ledger.on_event(&event(
            "Page.frameNavigated",
            Some("tab"),
            json!({"frame": {"id": "ad", "parentId": "main", "loaderId": "file"}}),
        ));
        assert_eq!(ledger.document_status("tab"), Some(503));
        ledger.on_event(&response("second", 200));
        ledger.on_event(&committed("second"));
        assert_eq!(ledger.document_status("tab"), Some(200));
        assert_eq!(ledger.document_status("other"), None);
        ledger.on_event(&event(
            "Target.detachedFromTarget",
            None,
            json!({"sessionId": "tab"}),
        ));
        assert_eq!(ledger.document_status("tab"), None);
    }

    #[test]
    fn downloads_and_document_requests_count_from_the_inputs_mark() {
        let mut ledger = Navigation::default();
        let begin = |guid: &str| {
            event(
                "Browser.downloadWillBegin",
                None,
                json!({"guid": guid, "url": "http://a.test/orders.csv",
                    "suggestedFilename": "orders.csv", "frameId": "T1"}),
            )
        };
        ledger.on_event(&begin("old"));
        ledger.input_sent();
        ledger.on_event(&event(
            "Network.requestWillBeSent",
            Some("tab"),
            json!({"requestId": "nav", "type": "Document"}),
        ));
        ledger.input_sent();
        let mark = ledger.take_mark().unwrap();
        assert_eq!(ledger.take_mark(), None);
        assert!(ledger.documents_open_since("tab", mark));
        assert!(ledger.downloads_since(mark).is_empty());
        ledger.on_event(&begin("new"));
        ledger.on_event(&begin("new"));
        ledger.on_event(&event(
            "Browser.downloadProgress",
            None,
            json!({"guid": "new", "state": "completed", "receivedBytes": 42,
                "filePath": "/tmp/jev-downloads/orders.csv"}),
        ));
        ledger.on_event(&event(
            "Network.loadingFailed",
            Some("tab"),
            json!({"requestId": "nav"}),
        ));
        assert!(!ledger.documents_open_since("tab", mark));
        let downloads = ledger.downloads_since(mark);
        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].name, "orders.csv");
        assert_eq!(downloads[0].frame, "T1");
        assert_eq!(downloads[0].state, DownloadState::Completed);
        assert_eq!(downloads[0].bytes, 42);
        assert_eq!(
            downloads[0].file_path.as_deref(),
            Some("/tmp/jev-downloads/orders.csv")
        );
    }
}
