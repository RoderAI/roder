//! The picture a native computer result carries when no screenshot could be
//! taken. A native call's output must be a screenshot: a result without one
//! cannot be replayed, and the thread stops there. So every path out of a
//! computer call, failed or not, ends in an image, and this one says it is a
//! stand-in.

use serde_json::json;

use super::client::cut;
use super::session::{DirectSession, DirectStep};

/// A 288x96 one-bit PNG reading "SCREENSHOT UNAVAILABLE" (243 bytes).
pub(crate) const SCREENSHOT_UNAVAILABLE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAASAAAABgAQAAAACV15L1AAAAuklEQVR42u2UUQrDMAiGhVyr4NF2tYDXkjl/I+nj8jTTESui8hXsT5VoxV721d4Hejak1JS4w5u7mGhGdGoggUs48s4yO6jrIJuQyAaQi4Z5U08WHdq2SsjGyHJ/gaW2xZCI8kaQCxh6ylyEHqU3y6BzDp8KKevFyr1pRENiiIREibgGauCGx2OapeGFGshtO4j7RYSuSzcgL1PMVgWtiFkBMW7rHlD8+XPk2IgUNpKxCL+Gzjn8a2jFPj2RV13wTwuDAAAAAElFTkSuQmCC";

/// What the note and the text say about a picture that is not one.
fn unavailable_text(reason: &str) -> String {
    format!("Screenshot unavailable ({reason}); the attached image is a placeholder.")
}

fn one_line(text: &str, chars: usize) -> String {
    cut(
        &text.split_whitespace().collect::<Vec<_>>().join(" "),
        chars,
    )
}

impl DirectStep {
    /// A failed step with the placeholder in place of a screenshot, for a
    /// call that failed before it could look at the page at all.
    pub(crate) fn screenshot_lost(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let note = unavailable_text(&one_line(&reason, 100));
        Self {
            text: format!("{reason}\n{note}"),
            data: json!({
                "error": reason,
                "screenshot_unavailable": true,
                "computer_notes": [note],
            }),
            is_error: true,
            image: Some(SCREENSHOT_UNAVAILABLE.to_string()),
            ..Self::default()
        }
    }
}

impl DirectSession {
    /// A screenshot of the tab or, when none can be taken (a password the
    /// page shows, a capture Chrome returned empty), the placeholder. It is
    /// not an error on its own: the actions before it worked. The data says
    /// so under `screenshot_unavailable` and gives the reason.
    pub(crate) async fn screen(&mut self) -> DirectStep {
        let mut step = self.run("screenshot", &json!({})).await;
        if step.image.is_some() {
            return step;
        }
        let reason = if step.data["screenshot_withheld"] == json!(true) {
            "a password or code typed on this page is still shown on it; clear that field or \
             leave the page"
                .to_string()
        } else {
            one_line(&step.text, 160)
        };
        let cleanup_error = step.data.get("cleanup_error").cloned();
        step.is_error = cleanup_error.is_some();
        step.text = unavailable_text(&reason);
        step.data = json!({
            "tool": "screenshot",
            "screenshot_unavailable": true,
            "screenshot_error": reason,
        });
        if let Some(error) = cleanup_error {
            step.data["cleanup_error"] = error;
        }
        step.image = Some(SCREENSHOT_UNAVAILABLE.to_string());
        step
    }
}

/// The note for a screenshot that is a placeholder, from its reason.
pub(crate) fn unavailable_note(reason: &str) -> String {
    unavailable_text(&one_line(reason, 100))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes() -> Vec<u8> {
        // Standard base64 decode, enough for a test: no dependency for it.
        let encoded = SCREENSHOT_UNAVAILABLE
            .strip_prefix("data:image/png;base64,")
            .unwrap();
        let mut out = Vec::new();
        let (mut bits, mut have) = (0u32, 0);
        for c in encoded.bytes().filter(|c| *c != b'=') {
            let value = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                other => panic!("not base64: {other}"),
            };
            bits = (bits << 6) | value as u32;
            have += 6;
            if have >= 8 {
                have -= 8;
                out.push((bits >> have) as u8);
                bits &= (1 << have) - 1;
            }
        }
        out
    }

    #[test]
    fn the_placeholder_is_a_small_valid_png() {
        let png = bytes();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        let size = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
        assert_eq!((size(16), size(20)), (288, 96));
        assert!(png.len() < 1024, "{} bytes", png.len());
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn a_lost_screenshot_is_a_failed_step_that_still_carries_the_placeholder() {
        let step = DirectStep::screenshot_lost("no browser is bound");
        assert!(step.is_error);
        assert_eq!(step.image.as_deref(), Some(SCREENSHOT_UNAVAILABLE));
        assert!(step.text.contains("no browser is bound"));
        assert_eq!(step.data["screenshot_unavailable"], true);
        assert_eq!(
            step.data["computer_notes"][0],
            "Screenshot unavailable (no browser is bound); the attached image is a placeholder."
        );
    }
}
