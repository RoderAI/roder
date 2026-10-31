//! The frames of another process each tab has, as DevTools sessions kept
//! for as long as the frame lives.
//!
//! A tab that turned on `Target.setAutoAttach` (see `page/frames.rs`) has
//! Chrome attach a flat session to each frame of another site (an
//! out-of-process iframe) the moment it appears, and detach it when the
//! frame goes or moves into the page's process. The ledger keeps those
//! sessions from the events the connection reads anyway, so reading a
//! frame's text is one evaluate on a session already open, not a target
//! listing, an attach and a detach on every observation. It also remembers
//! when a frame last missed its read deadline, so a frame that stopped
//! answering costs a short wait on later reads, not the full one; the note
//! goes with the session. The rule follows fastbrowse's browser session
//! (MIT).

use std::time::Instant;

use serde_json::Value;

/// One frame of another process, attached under its tab's session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameSession {
    /// The frame's target id, which is also its frame id.
    pub(crate) target: String,
    /// The flat session Chrome attached to it.
    pub(crate) session: String,
    /// When a read of it last ran past its deadline, until one answers.
    pub(crate) missed: Option<Instant>,
}

#[derive(Default)]
pub(super) struct Frames {
    /// (parent session, frame), in the order the frames attached.
    attached: Vec<(String, FrameSession)>,
}

impl Frames {
    /// Keep the ledger from one DevTools event.
    pub(super) fn on_event(&mut self, event: &Value) {
        let params = &event["params"];
        match (event["method"].as_str(), event["sessionId"].as_str()) {
            (Some("Target.attachedToTarget"), Some(parent))
                if params["targetInfo"]["type"] == "iframe" =>
            {
                let (Some(target), Some(session)) = (
                    params["targetInfo"]["targetId"].as_str(),
                    params["sessionId"].as_str(),
                ) else {
                    return;
                };
                // A frame attached again (its process swapped) keeps its
                // place under its new session.
                let frame = FrameSession {
                    target: target.to_string(),
                    session: session.to_string(),
                    missed: None,
                };
                match self
                    .attached
                    .iter_mut()
                    .find(|(_, known)| known.target == target)
                {
                    Some(entry) => *entry = (parent.to_string(), frame),
                    None => self.attached.push((parent.to_string(), frame)),
                }
            }
            // A frame that went, and every frame of a tab that went.
            (Some("Target.detachedFromTarget"), _) => {
                let gone = params["sessionId"].as_str().unwrap_or_default();
                self.attached
                    .retain(|(parent, frame)| frame.session != gone && parent != gone);
            }
            _ => {}
        }
    }

    /// The frames attached under `parent`, oldest first.
    pub(super) fn of(&self, parent: &str) -> Vec<FrameSession> {
        self.attached
            .iter()
            .filter(|(known, _)| known == parent)
            .map(|(_, frame)| frame.clone())
            .collect()
    }

    /// Note that a read of the frame on `session` ran past its deadline at
    /// `at`, or (`None`) that one answered.
    pub(super) fn set_missed(&mut self, session: &str, at: Option<Instant>) {
        if let Some((_, frame)) = self
            .attached
            .iter_mut()
            .find(|(_, frame)| frame.session == session)
        {
            frame.missed = at;
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn attached(parent: &str, target: &str, session: &str, kind: &str) -> Value {
        json!({
            "method": "Target.attachedToTarget",
            "sessionId": parent,
            "params": {
                "sessionId": session,
                "targetInfo": {"targetId": target, "type": kind},
            },
        })
    }

    fn detached(session: &str) -> Value {
        json!({"method": "Target.detachedFromTarget", "params": {"sessionId": session}})
    }

    fn sessions(frames: &Frames, parent: &str) -> Vec<String> {
        frames
            .of(parent)
            .into_iter()
            .map(|frame| frame.session)
            .collect()
    }

    #[test]
    fn frames_are_kept_per_tab_until_they_or_their_tab_go() {
        let mut frames = Frames::default();
        frames.on_event(&attached("tab", "ad", "s-ad", "iframe"));
        frames.on_event(&attached("tab", "widget", "s-widget", "iframe"));
        frames.on_event(&attached("other", "map", "s-map", "iframe"));
        // A worker, or a tab the browser session attached, is no frame.
        frames.on_event(&attached("tab", "w", "s-w", "worker"));
        frames.on_event(&json!({
            "method": "Target.attachedToTarget",
            "params": {"sessionId": "s-tab", "targetInfo": {"targetId": "t", "type": "iframe"}},
        }));
        assert_eq!(sessions(&frames, "tab"), ["s-ad", "s-widget"]);
        assert_eq!(sessions(&frames, "other"), ["s-map"]);

        // Re-attached under a new session, the frame keeps its place.
        frames.on_event(&attached("tab", "ad", "s-ad2", "iframe"));
        assert_eq!(sessions(&frames, "tab"), ["s-ad2", "s-widget"]);

        frames.on_event(&detached("s-widget"));
        assert_eq!(sessions(&frames, "tab"), ["s-ad2"]);
        frames.on_event(&detached("other"));
        assert!(frames.of("other").is_empty());
    }

    #[test]
    fn a_missed_deadline_is_noted_until_a_read_answers() {
        let mut frames = Frames::default();
        frames.on_event(&attached("tab", "ad", "s-ad", "iframe"));
        let at = Instant::now();
        frames.set_missed("s-ad", Some(at));
        frames.set_missed("unknown", Some(at));
        assert_eq!(frames.of("tab")[0].missed, Some(at));
        frames.set_missed("s-ad", None);
        assert_eq!(frames.of("tab")[0].missed, None);
        // A new session for the frame starts with a clean slate.
        frames.set_missed("s-ad", Some(at));
        frames.on_event(&attached("tab", "ad", "s-ad2", "iframe"));
        assert_eq!(frames.of("tab")[0].missed, None);
    }
}
