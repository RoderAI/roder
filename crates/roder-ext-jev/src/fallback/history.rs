//! Compact stale control trees while preserving bounded text evidence from prior pages.
use roder_api::transcript::{TranscriptItem, VIEW_IMAGE_DISPLAY_KEY};
const KEPT_READS: usize = 2;

/// Keep the latest [`KEPT_READS`] intact, retain bounded older text evidence,
/// and show only the newest screenshot.
pub(crate) fn compact(transcript: &mut [TranscriptItem]) {
    let mut reads = 0;
    let mut pictures = 0;
    let mut evidence_left = 8000;
    for item in transcript.iter_mut().rev() {
        let TranscriptItem::ToolResult(result) = item else {
            continue;
        };
        if let Some(payload) = result.display_payload.as_mut()
            && payload.get(VIEW_IMAGE_DISPLAY_KEY).is_some()
        {
            pictures += 1;
            if pictures > 1
                && let Some(payload) = payload.as_object_mut()
            {
                payload.remove(VIEW_IMAGE_DISPLAY_KEY);
                result.result = format!("{} [picture no longer shown]", first_line(&result.result));
            }
            continue;
        }
        if result.result.lines().count() <= 1 {
            continue;
        }
        reads += 1;
        if reads > KEPT_READS {
            let retained = result.result.split_once("\nText:\n").map(|(_, text)| text);
            if let Some(text) = retained.filter(|text| text.chars().count() <= evidence_left) {
                evidence_left -= text.chars().count();
                let source = result
                    .result
                    .lines()
                    .find(|line| line.starts_with("Page (untrusted"))
                    .unwrap_or("Earlier page observation (untrusted)");
                result.result = format!(
                    "{} [historical observation; controls elided]\n{}\nText:\n{}",
                    first_line(&result.result)
                        .split(" [historical observation;")
                        .next()
                        .unwrap(),
                    source,
                    text
                );
            } else {
                result.result = format!(
                    "{} [page read elided; earlier evidence unavailable]",
                    first_line(&result.result)
                );
            }
        }
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}
