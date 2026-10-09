// A scripted stand-in for the paired extension: each command is answered
// with the next reply queued for its kind, and what was sent, and when, is
// kept. Shapes follow the extension's wire results: a `page/snapshot` answer
// is `{ok, snapshot}`; an action of the build with `action-observation` is
// `{action, observation, tabId, untrusted}`; an older build answers an action
// with only what it did (`{ok, ref}`).
mod scripted {
    // Each test binary that includes this uses some of it.
    #![allow(dead_code)]

    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;
    use std::time::Instant;

    use roder_api::chrome::{
        ChromeCommand, ChromeController, ChromeError, ChromePermissionMode, ChromeStatus,
    };
    use serde_json::{Value, json};

    /// One command the extension was sent.
    #[derive(Clone)]
    pub struct Sent {
        pub kind: String,
        pub params: Value,
        /// When the command reached the extension.
        pub sent_at: Instant,
        /// When the extension answered it.
        pub answered_at: Instant,
    }

    #[derive(Default)]
    pub struct Scripted {
        replies: Mutex<HashMap<String, VecDeque<Result<Value, ChromeError>>>>,
        log: Mutex<Vec<Sent>>,
    }

    impl Scripted {
        pub fn reply(&self, kind: &str, reply: Result<Value, ChromeError>) {
            self.replies
                .lock()
                .unwrap()
                .entry(kind.to_string())
                .or_default()
                .push_back(reply);
        }

        pub fn log(&self) -> Vec<Sent> {
            self.log.lock().unwrap().clone()
        }

        pub fn kinds(&self) -> Vec<String> {
            self.log().into_iter().map(|sent| sent.kind).collect()
        }
    }

    #[async_trait::async_trait]
    impl ChromeController for Scripted {
        fn status(&self) -> ChromeStatus {
            ChromeStatus {
                connected: true,
                client_count: 1,
                enabled: true,
                mode: ChromePermissionMode::Control,
                ..ChromeStatus::default()
            }
        }
        fn set_enabled(&self, _: bool) {}
        fn set_mode(&self, _: ChromePermissionMode) {}
        async fn dispatch(&self, command: ChromeCommand) -> Result<Value, ChromeError> {
            let sent_at = Instant::now();
            let reply = self
                .replies
                .lock()
                .unwrap()
                .get_mut(&command.kind)
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| panic!("no reply scripted for {}", command.kind));
            self.log.lock().unwrap().push(Sent {
                kind: command.kind,
                params: command.params,
                sent_at,
                answered_at: Instant::now(),
            });
            reply
        }
    }

    /// A control as the extension's snapshot lists it.
    pub fn control(reference: &str, tag: &str, role: &str, text: &str) -> Value {
        json!({
            "ref": reference, "tag": tag, "text": text, "role": role,
            "selector": format!("#{reference}"),
            "box": {"x": 10, "y": 20, "width": 80, "height": 24},
        })
    }

    /// A page snapshot, with the extension's key order (text before controls).
    pub fn snapshot(url: &str, title: &str, text: &str, controls: Vec<Value>) -> Value {
        json!({
            "title": title, "url": url, "text": text, "controls": controls,
            "viewport": {"width": 1280, "height": 800, "scrollX": 0, "scrollY": 0,
                "devicePixelRatio": 1},
            "capturedAt": 1, "untrusted": true,
        })
    }

    /// What `page/snapshot` answers.
    pub fn snapshot_reply(snapshot: &Value) -> Value {
        json!({"ok": true, "snapshot": snapshot})
    }

    /// What an action answers on a build that observes the page after it.
    pub fn observed_reply(action: Value, observation: &Value, tab_id: u64) -> Value {
        json!({"action": action, "observation": observation, "tabId": tab_id, "untrusted": true})
    }
}
