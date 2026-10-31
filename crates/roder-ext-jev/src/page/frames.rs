//! Reading, never acting in, frames the page cannot reach.
//!
//! `snapshot.js` observes same-origin frames like the document itself, but a
//! frame of another origin (a booking or payment widget drawn over the page)
//! is out of its reach, so a reservation panel that opened after a click was
//! invisible and Jev judged the goal against the list behind it. Here each
//! large, visible frame of another origin is read over DevTools: one in the
//! page's own process through an isolated world, one in another process
//! (another site) through the session Chrome attached to it when it
//! appeared (`Target.setAutoAttach`, kept by the connection's frame ledger,
//! `cdp/frames.rs`), so a read is one evaluate, not a target listing, an
//! attach and a detach. At most two frames, 700 characters each, are added
//! to the observation as `[frame <origin>]` lines of page text, so the model
//! can judge what it shows, and as `frames` for the result. Their controls
//! are deliberately not offered: Jev reads such a widget but cannot press
//! its buttons. `JEV_FRAME_TEXT=0` turns this off.
//!
//! Each frame's read has its own deadline, 5 s, not the connection's 30 s:
//! an advertisement whose renderer stopped answering would otherwise hold
//! every observation for the whole call timeout. A frame of another process
//! that misses it is left out and gets 0.5 s on later reads, until one
//! answers or 30 s have passed (a frame that recovered but answers slowly
//! is given the full deadline again). The rule follows fastbrowse's page
//! reads (MIT).

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Page, TEXT_CHARS};
use crate::cdp::FrameSession;
use crate::engine::JevFrameText;

/// Frames read per observation.
const FRAMES: usize = 2;
/// Characters kept of each frame's text.
pub(crate) const FRAME_CHARS: usize = 700;
/// How long one frame's read may take before the observation goes on
/// without it.
pub(crate) const FRAME_READ: Duration = Duration::from_secs(5);
/// What a frame of another process that missed [`FRAME_READ`] gets on later
/// reads, until one answers or [`FRAME_RECHECK`] has passed.
pub(crate) const FRAME_RETRY: Duration = Duration::from_millis(500);
/// How long a frame that missed its deadline keeps the short one.
const FRAME_RECHECK: Duration = Duration::from_secs(30);

/// Whether a frame's element is shown, at least 200 by 100 pixels, and on
/// screen; run on the element.
const FRAME_SHOWN_JS: &str = "function(){const r=this.getBoundingClientRect();\
     return this.checkVisibility({checkOpacity:true,checkVisibilityCSS:true})&&\
     r.width>=200&&r.height>=100&&r.bottom>0&&r.right>0&&r.top<innerHeight&&r.left<innerWidth;}";
/// A frame document's text: lines trimmed, blank ones dropped, joined.
const FRAME_TEXT_JS: &str = "(()=>(document.body?.innerText||'').split('\\n')\
     .map(l=>l.replace(/\\s+/g,' ').trim()).filter(Boolean).join(' · ').slice(0,700)\
     .toWellFormed())()";
const OBJECT_GROUP: &str = "jev-frames";

/// Where a frame of another origin is read.
enum Reach {
    /// In the page's process: its frame id and origin.
    Here(String, String),
    /// In another process, through its own session.
    Elsewhere(FrameSession),
}

/// Whether frame text is read: `JEV_FRAME_TEXT`, on unless it is `0`,
/// `false`, `no` or `off`.
pub(crate) fn frame_text_on() -> bool {
    std::env::var("JEV_FRAME_TEXT").map_or(true, |value| {
        !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
    })
}

/// The deadline for a frame's read: the short one while a recent read of it
/// missed the full one.
fn read_budget(missed: Option<Instant>, now: Instant) -> Duration {
    match missed {
        Some(at) if now.saturating_duration_since(at) < FRAME_RECHECK => FRAME_RETRY,
        _ => FRAME_READ,
    }
}

impl Page {
    /// Read frame text or not, whatever `JEV_FRAME_TEXT` says.
    #[cfg(test)]
    pub(crate) fn set_frame_text(&mut self, on: bool) {
        self.frame_text = on;
    }

    /// The sessions of the current tab's frames of another process.
    #[cfg(test)]
    pub(crate) fn frame_sessions(&self) -> Vec<String> {
        self.connection
            .frame_sessions(&self.tab().session)
            .into_iter()
            .map(|frame| frame.session)
            .collect()
    }

    /// Have Chrome attach a session to each frame of another process the
    /// tab has or opens, kept until the frame goes (see `cdp/frames.rs`).
    /// Workers and other targets are left alone. Best effort: a browser
    /// that refuses only leaves such frames unread.
    pub(super) async fn follow_frames(&mut self) {
        if !self.frame_text {
            return;
        }
        self.call(
            "Target.setAutoAttach",
            json!({"autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true,
                   "filter": [{"type": "iframe", "exclude": false}, {"exclude": true}]}),
        )
        .await
        .ok();
    }

    /// The text of up to two large, visible frames of another origin, best
    /// effort: a frame that cannot be read in time is left out. Frames in
    /// the page's process are its frame tree's children; frames in another
    /// process are the sessions Chrome attached under the tab.
    pub(super) async fn read_frames(&mut self) -> Vec<JevFrameText> {
        let Ok(tree) = self.call("Page.getFrameTree", json!({})).await else {
            return Vec::new();
        };
        let tree = &tree["frameTree"];
        let origin = tree["frame"]["securityOrigin"].clone();
        let session = self.tab().session.clone();
        let elsewhere = self.connection.frame_sessions(&session);
        let mut candidates = tree["childFrames"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|child| &child["frame"])
            // A same-origin frame's text is already page text.
            .filter(|frame| frame["securityOrigin"] != origin)
            .filter_map(|frame| Some((frame["id"].as_str()?, frame_origin(frame))))
            // A frame of another process is read through its session.
            .filter(|(id, _)| !elsewhere.iter().any(|frame| frame.target == *id))
            .map(|(id, origin)| Reach::Here(id.to_string(), origin))
            .collect::<Vec<_>>();
        candidates.extend(elsewhere.into_iter().map(Reach::Elsewhere));
        let mut read = Vec::new();
        for candidate in candidates {
            if read.len() >= FRAMES {
                break;
            }
            let id = match &candidate {
                Reach::Here(id, _) => id.clone(),
                Reach::Elsewhere(frame) => frame.target.clone(),
            };
            if !self.frame_shown(&id).await {
                continue;
            }
            let text = match candidate {
                Reach::Here(id, origin) => {
                    self.frame_text_here(&id).await.map(|text| (origin, text))
                }
                Reach::Elsewhere(frame) => self.frame_text_elsewhere(&frame).await,
            };
            if let Some((origin, text)) = text.filter(|(_, text)| !text.is_empty()) {
                read.push(JevFrameText {
                    origin,
                    text: text.chars().take(FRAME_CHARS).collect(),
                });
            }
        }
        self.call(
            "Runtime.releaseObjectGroup",
            json!({"objectGroup": OBJECT_GROUP}),
        )
        .await
        .ok();
        read
    }

    /// Whether the frame's element in the page is shown and large enough.
    async fn frame_shown(&mut self, frame: &str) -> bool {
        let shown = async {
            let owner = self
                .call("DOM.getFrameOwner", json!({"frameId": frame}))
                .await?;
            let node = self
                .call(
                    "DOM.resolveNode",
                    json!({"backendNodeId": owner["backendNodeId"], "objectGroup": OBJECT_GROUP}),
                )
                .await?;
            let shown = self
                .call(
                    "Runtime.callFunctionOn",
                    json!({"objectId": node["object"]["objectId"],
                           "functionDeclaration": FRAME_SHOWN_JS, "returnByValue": true}),
                )
                .await?;
            anyhow::Ok(shown["result"]["value"] == json!(true))
        };
        shown.await.unwrap_or(false)
    }

    /// A frame in the page's own process, read in an isolated world so the
    /// frame's scripts cannot see or change the read, within
    /// [`FRAME_READ`].
    async fn frame_text_here(&mut self, frame: &str) -> Option<String> {
        let session = self.tab().session.clone();
        let world = self
            .connection
            .call_within(
                "Page.createIsolatedWorld",
                json!({"frameId": frame, "worldName": "jev-frame-text"}),
                Some(&session),
                FRAME_READ,
            )
            .await
            .ok()?;
        let context = world["executionContextId"].as_i64()?;
        let read = self
            .connection
            .call_within(
                "Runtime.evaluate",
                json!({"expression": FRAME_TEXT_JS, "contextId": context,
                       "returnByValue": true}),
                Some(&session),
                FRAME_READ,
            )
            .await
            .ok()?;
        read["result"]["value"].as_str().map(str::to_string)
    }

    /// A frame in another process, on the session Chrome attached to it,
    /// within its deadline (see [`read_budget`]): its origin, from the
    /// address the same evaluate read, and its text.
    async fn frame_text_elsewhere(&mut self, frame: &FrameSession) -> Option<(String, String)> {
        let started = Instant::now();
        let budget = read_budget(frame.missed, started);
        let expression = format!("[location.href,{FRAME_TEXT_JS}]");
        let read = self
            .connection
            .call_within(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true}),
                Some(&frame.session),
                budget,
            )
            .await;
        let Ok(read) = read else {
            // Only a full miss starts the short deadline's 30 s, so a frame
            // still stuck after them is given the full one again.
            if budget == FRAME_READ && started.elapsed() >= budget {
                self.connection
                    .set_frame_missed(&frame.session, Some(started));
            }
            return None;
        };
        self.connection.set_frame_missed(&frame.session, None);
        let value = &read["result"]["value"];
        Some((
            url_origin(value[0].as_str()?),
            value[1].as_str()?.to_string(),
        ))
    }
}

/// A frame's origin as the result names it; an opaque (sandboxed) frame by
/// its address instead.
fn frame_origin(frame: &Value) -> String {
    match frame["securityOrigin"].as_str() {
        Some(origin) if origin.contains("://") && !origin.ends_with("://") => origin.to_string(),
        _ => url_origin(frame["url"].as_str().unwrap_or_default()),
    }
}

/// An address's origin, or the address itself (`about:srcdoc`) when it has
/// none.
fn url_origin(url: &str) -> String {
    match reqwest::Url::parse(url).map(|url| url.origin()) {
        Ok(origin) if origin.is_tuple() => origin.ascii_serialization(),
        _ => url.chars().take(80).collect(),
    }
}

/// Put the frames' text on an observation: as `frames`, and as one
/// `[frame <origin>]` line each after the page's own text, which is cut
/// first to keep the whole within the page-text cap.
pub(super) fn add_frames(observation: &mut Value, frames: Vec<JevFrameText>) {
    if frames.is_empty() {
        return;
    }
    let lines = frames
        .iter()
        .map(|frame| format!("[frame {}] {}", frame.origin, frame.text))
        .collect::<Vec<_>>()
        .join("\n");
    let room = TEXT_CHARS.saturating_sub(lines.chars().count() + 1);
    let page = observation["text"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(room)
        .collect::<String>();
    let text = match page.is_empty() {
        true => lines,
        false => format!("{page}\n{lines}"),
    };
    observation["text"] = json!(text);
    observation["frames"] = json!(frames);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(origin: &str, text: &str) -> JevFrameText {
        JevFrameText {
            origin: origin.into(),
            text: text.into(),
        }
    }

    #[test]
    fn frame_text_follows_the_page_text_within_the_cap() {
        let mut page = json!({"text": "Results\nAngie's Pizza"});
        add_frames(&mut page, Vec::new());
        assert_eq!(page, json!({"text": "Results\nAngie's Pizza"}));
        add_frames(
            &mut page,
            vec![frame(
                "https://widgets.test",
                "Complete Your Reservation · 3 Guests",
            )],
        );
        assert_eq!(
            page["text"],
            "Results\nAngie's Pizza\n[frame https://widgets.test] Complete Your Reservation · 3 Guests"
        );
        assert_eq!(page["frames"][0]["origin"], "https://widgets.test");

        let mut long = json!({"text": "x".repeat(TEXT_CHARS)});
        add_frames(&mut long, vec![frame("https://w.test", "Reserve Now")]);
        let text = long["text"].as_str().unwrap();
        assert_eq!(text.chars().count(), TEXT_CHARS);
        assert!(
            text.ends_with("\n[frame https://w.test] Reserve Now"),
            "{text}"
        );
    }

    #[test]
    fn a_frame_that_missed_its_deadline_gets_the_short_one_for_30_s() {
        let now = Instant::now();
        assert_eq!(read_budget(None, now), FRAME_READ);
        assert_eq!(read_budget(Some(now), now), FRAME_RETRY);
        let earlier = now - Duration::from_secs(29);
        assert_eq!(read_budget(Some(earlier), now), FRAME_RETRY);
        let long_ago = now - FRAME_RECHECK;
        assert_eq!(read_budget(Some(long_ago), now), FRAME_READ);
    }

    #[test]
    fn an_opaque_frame_is_named_by_its_address() {
        assert_eq!(
            frame_origin(&json!({"securityOrigin": "http://localhost:4000", "url": "x"})),
            "http://localhost:4000"
        );
        assert_eq!(
            frame_origin(&json!({"securityOrigin": "://", "url": "about:srcdoc"})),
            "about:srcdoc"
        );
        assert_eq!(
            frame_origin(&json!({"securityOrigin": "null", "url": "about:srcdoc"})),
            "about:srcdoc"
        );
        assert_eq!(
            url_origin("http://localhost:4000/pages/w.html?a=1"),
            "http://localhost:4000"
        );
        assert_eq!(url_origin("about:srcdoc"), "about:srcdoc");
    }
}
