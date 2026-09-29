//! Reading, never acting in, frames the page cannot reach.
//!
//! `snapshot.js` observes same-origin frames like the document itself, but a
//! frame of another origin (a booking or payment widget drawn over the page)
//! is out of its reach, so a reservation panel that opened after a click was
//! invisible and Jev judged the goal against the list behind it. Here each
//! large, visible frame of another origin is read over DevTools: one in the
//! page's own process through an isolated world, one in another process
//! (another site) by attaching to its target for one evaluate. At most two
//! frames, 700 characters each, are added to the observation as
//! `[frame <origin>]` lines of page text, so the model can judge what it
//! shows, and as `frames` for the result. Their controls are deliberately
//! not offered: Jev reads such a widget but cannot press its buttons.
//! `JEV_FRAME_TEXT=0` turns this off.

use serde_json::{Value, json};

use super::{Page, TEXT_CHARS};
use crate::engine::JevFrameText;

/// Frames read per observation.
const FRAMES: usize = 2;
/// Characters kept of each frame's text.
pub(crate) const FRAME_CHARS: usize = 700;

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

impl Page {
    /// Read frame text or not, whatever `JEV_FRAME_TEXT` says.
    #[cfg(test)]
    pub(crate) fn set_frame_text(&mut self, on: bool) {
        self.frame_text = on;
    }

    /// The text of up to two large, visible frames of another origin, best
    /// effort: a frame that cannot be read is left out. Frames in the page's
    /// process are its frame tree's children; frames in another process are
    /// `iframe` targets whose parent is the page's main frame.
    pub(super) async fn read_frames(&mut self) -> Vec<JevFrameText> {
        let Ok(tree) = self.call("Page.getFrameTree", json!({})).await else {
            return Vec::new();
        };
        let tree = &tree["frameTree"];
        let main = tree["frame"]["id"].clone();
        let origin = tree["frame"]["securityOrigin"].clone();
        // (frame id, origin, in this process)
        let mut candidates = tree["childFrames"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|child| &child["frame"])
            // A same-origin frame's text is already page text.
            .filter(|frame| frame["securityOrigin"] != origin)
            .filter_map(|frame| {
                Some((frame["id"].as_str()?.to_string(), frame_origin(frame), true))
            })
            .collect::<Vec<_>>();
        if let Ok(targets) = self
            .connection
            .call("Target.getTargets", json!({}), None)
            .await
        {
            candidates.extend(
                targets["targetInfos"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|target| target["type"] == "iframe" && target["parentFrameId"] == main)
                    .filter_map(|target| {
                        let url = target["url"].as_str()?;
                        Some((
                            target["targetId"].as_str()?.to_string(),
                            url_origin(url),
                            false,
                        ))
                    }),
            );
        }
        let mut read = Vec::new();
        for (id, origin, here) in candidates {
            if read.len() >= FRAMES {
                break;
            }
            if !self.frame_shown(&id).await {
                continue;
            }
            let text = match here {
                true => self.frame_text_here(&id).await,
                false => self.frame_text_elsewhere(&id).await,
            };
            if let Some(text) = text.filter(|text| !text.is_empty()) {
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
    /// frame's scripts cannot see or change the read.
    async fn frame_text_here(&mut self, frame: &str) -> Option<String> {
        let world = self
            .call(
                "Page.createIsolatedWorld",
                json!({"frameId": frame, "worldName": "jev-frame-text"}),
            )
            .await
            .ok()?;
        let context = world["executionContextId"].as_i64()?;
        let read = self
            .call(
                "Runtime.evaluate",
                json!({"expression": FRAME_TEXT_JS, "contextId": context,
                       "returnByValue": true}),
            )
            .await
            .ok()?;
        read["result"]["value"].as_str().map(str::to_string)
    }

    /// A frame in another process: its own DevTools target, whose id is the
    /// frame's, attached for one read and detached again.
    async fn frame_text_elsewhere(&mut self, frame: &str) -> Option<String> {
        let attached = self
            .connection
            .call(
                "Target.attachToTarget",
                json!({"targetId": frame, "flatten": true}),
                None,
            )
            .await
            .ok()?;
        let session = attached["sessionId"].as_str()?.to_string();
        let read = self
            .connection
            .call(
                "Runtime.evaluate",
                json!({"expression": FRAME_TEXT_JS, "returnByValue": true}),
                Some(&session),
            )
            .await;
        self.connection
            .call(
                "Target.detachFromTarget",
                json!({"sessionId": session}),
                None,
            )
            .await
            .ok();
        read.ok()?["result"]["value"].as_str().map(str::to_string)
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
