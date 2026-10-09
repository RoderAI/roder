//! Tool-result images for engines that cannot receive them.
//!
//! Browser and desktop tools attach a screenshot to their result under
//! `__view_image`, and their text says it is attached. Only engines that
//! answer true to [`InferenceEngine::tool_result_image_input`] forward it. For
//! every other engine the request carries a copy of the transcript without the
//! image and with one notice line on that result, screenshot-only tools are not
//! offered, and prompt accounting charges no image tokens for it. The stored
//! transcript is never touched, so a thread moved to another engine replays
//! the same history.

use roder_api::inference::InferenceEngine;
use roder_api::transcript::{ToolResultRecord, TranscriptItem, VIEW_IMAGE_DISPLAY_KEY};
use serde_json::Value;

use crate::Runtime;

/// Appended, on its own line, to each tool result whose image was left out.
pub(crate) const IMAGE_WITHHELD_NOTICE: &str =
    "screenshot not shown: this model cannot receive images in tool results";

/// Tools whose only product is an image. An engine that cannot receive tool
/// images gets nothing from them, so they are not offered to it. Tools that
/// merely attach a screenshot to another result stay: their text still works.
/// `computer` has its own rule (`native_tool_supported`).
const IMAGE_ONLY_TOOLS: &[&str] = &[
    "browser_use_screenshot",
    "chrome_screenshot",
    "jev_tab_screenshot",
    "view_image",
];

/// Whether the engine serving a request receives tool-result images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolImages {
    Forwarded,
    Withheld,
}

impl ToolImages {
    pub(crate) fn for_engine(engine: &dyn InferenceEngine, model: &str) -> Self {
        if engine.tool_result_image_input(model) {
            Self::Forwarded
        } else {
            Self::Withheld
        }
    }

    /// False for a tool that would give this engine nothing.
    pub(crate) fn offers(self, tool: &str) -> bool {
        self == Self::Forwarded || !IMAGE_ONLY_TOOLS.contains(&tool)
    }
}

impl Runtime {
    /// What the engine registered for `provider` does with tool-result images.
    /// A provider with no registered engine keeps today's behaviour (images
    /// forwarded, tools offered): it cannot serve a request anyway.
    pub(crate) fn tool_images(&self, provider: &str, model: &str) -> ToolImages {
        self.engine_for(provider)
            .map(|engine| ToolImages::for_engine(engine.as_ref(), model))
            .unwrap_or(ToolImages::Forwarded)
    }
}

pub(crate) fn carries_image(result: &ToolResultRecord) -> bool {
    result
        .display_payload
        .as_ref()
        .is_some_and(|payload| payload.get(VIEW_IMAGE_DISPLAY_KEY).is_some())
}

/// The transcript as the engine should see it. Identical to `items` when the
/// engine receives tool-result images; otherwise each result that carries one
/// loses it and gets the notice. Tool calls and their results all stay, so
/// history that used a tool the engine is no longer offered still replays.
pub(crate) fn request_transcript(
    items: &[TranscriptItem],
    images: ToolImages,
) -> Vec<TranscriptItem> {
    items
        .iter()
        .map(|item| match item {
            TranscriptItem::ToolResult(result)
                if images == ToolImages::Withheld && carries_image(result) =>
            {
                TranscriptItem::ToolResult(without_image(result))
            }
            other => other.clone(),
        })
        .collect()
}

fn without_image(result: &ToolResultRecord) -> ToolResultRecord {
    let mut display_payload = result.display_payload.clone();
    if let Some(Value::Object(fields)) = &mut display_payload {
        fields.remove(VIEW_IMAGE_DISPLAY_KEY);
        if fields.is_empty() {
            display_payload = None;
        }
    }
    let text = if result.result.is_empty() {
        IMAGE_WITHHELD_NOTICE.to_string()
    } else {
        format!("{}\n{IMAGE_WITHHELD_NOTICE}", result.result)
    };
    ToolResultRecord {
        id: result.id.clone(),
        name: result.name.clone(),
        result: text,
        display_payload,
        is_error: result.is_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(text: &str, payload: Option<Value>) -> TranscriptItem {
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "call".into(),
            name: Some("browser_use_click".into()),
            result: text.into(),
            display_payload: payload,
            is_error: false,
        })
    }

    fn shot() -> Value {
        json!({"__view_image": {"image_url": "data:image/png;base64,YWJj", "detail": "original"}})
    }

    fn text_of(item: &TranscriptItem) -> &str {
        match item {
            TranscriptItem::ToolResult(result) => &result.result,
            other => panic!("not a tool result: {other:?}"),
        }
    }

    #[test]
    fn forwarded_engines_get_the_transcript_unchanged() {
        let items = vec![result("clicked", Some(shot()))];
        assert_eq!(request_transcript(&items, ToolImages::Forwarded), items);
    }

    #[test]
    fn withheld_drops_only_the_image_key_and_adds_one_notice_line() {
        let mut payload = shot();
        payload["path"] = json!("board.png");
        let items = vec![result("clicked. Screenshot attached.", Some(payload))];
        let copy = request_transcript(&items, ToolImages::Withheld);
        assert_eq!(
            text_of(&copy[0]),
            format!("clicked. Screenshot attached.\n{IMAGE_WITHHELD_NOTICE}")
        );
        let TranscriptItem::ToolResult(copy) = &copy[0] else {
            unreachable!()
        };
        assert_eq!(copy.display_payload, Some(json!({"path": "board.png"})));
        assert!(!serde_json::to_string(copy).unwrap().contains("data:image"));
        // The input is the stored transcript; it must be untouched.
        assert_eq!(text_of(&items[0]), "clicked. Screenshot attached.");
        assert!(carries_image(match &items[0] {
            TranscriptItem::ToolResult(result) => result,
            _ => unreachable!(),
        }));
    }

    #[test]
    fn withheld_leaves_results_without_an_image_alone() {
        let items = vec![
            result("no picture", None),
            result("other payload", Some(json!({"path": "a"}))),
            TranscriptItem::AssistantMessage(roder_api::transcript::AssistantMessage {
                text: "ok".into(),
                phase: None,
            }),
        ];
        assert_eq!(request_transcript(&items, ToolImages::Withheld), items);
    }

    #[test]
    fn an_empty_result_becomes_just_the_notice_and_an_empty_payload_is_dropped() {
        let copy = request_transcript(&[result("", Some(shot()))], ToolImages::Withheld);
        assert_eq!(text_of(&copy[0]), IMAGE_WITHHELD_NOTICE);
        let TranscriptItem::ToolResult(copy) = &copy[0] else {
            unreachable!()
        };
        assert_eq!(copy.display_payload, None);
    }

    #[test]
    fn only_image_only_tools_are_hidden_and_only_when_withheld() {
        for tool in IMAGE_ONLY_TOOLS {
            assert!(ToolImages::Forwarded.offers(tool));
            assert!(!ToolImages::Withheld.offers(tool), "{tool}");
        }
        for tool in ["browser_use_click", "chrome_click", "jev_tab_look", "shell"] {
            assert!(ToolImages::Withheld.offers(tool), "{tool}");
        }
    }
}
