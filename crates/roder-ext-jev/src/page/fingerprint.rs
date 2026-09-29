//! A page's fingerprint: what the stall rule compares between steps.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::engine::JevFrameText;
use crate::python_json;

/// A page's fingerprint without frames, as the loop takes it.
#[cfg(test)]
pub(crate) fn fingerprint(observation: &Value) -> String {
    hash(fingerprint_content(observation))
}

/// The fingerprint of a page with frames of another origin read: the
/// frames count by their origins, never by their text. Large visible frames
/// are often ads, video and chat widgets whose text rotates by itself, and
/// hashing it would make every no-op step look like progress and defeat
/// the stall rule; a widget that opens or closes still changes the set.
/// Without frames it is [`fingerprint`].
pub(crate) fn fingerprint_with_frames(observation: &Value, frames: &[JevFrameText]) -> String {
    let mut content = fingerprint_content(observation);
    if !frames.is_empty() {
        let origins = frames
            .iter()
            .map(|frame| Value::String(frame.origin.clone()))
            .collect();
        content.insert("frames".into(), Value::Array(origins));
    }
    hash(content)
}

/// Upstream hashes a canonical dump of exactly these four fields. Jev hashes
/// only the actions upstream would have observed: offscreen controls and the
/// `context` and `section` naming each one are left out, so a countdown or
/// carousel below the fold cannot make a no-op step look like progress and
/// defeat the stall rule. An observation without either hashes exactly as
/// upstream's does.
fn fingerprint_content(observation: &Value) -> serde_json::Map<String, Value> {
    let mut content = serde_json::Map::new();
    for key in ["url", "text", "scroll"] {
        content.insert(
            key.into(),
            observation.get(key).cloned().unwrap_or(Value::Null),
        );
    }
    let actions = observation.get("actions").map(|actions| match actions {
        Value::Array(actions) => Value::Array(
            actions
                .iter()
                .filter(|action| action.get("offscreen") != Some(&Value::Bool(true)))
                .map(|action| {
                    let mut action = action.clone();
                    if let Some(action) = action.as_object_mut() {
                        action.remove("context");
                        action.remove("section");
                    }
                    action
                })
                .collect(),
        ),
        other => other.clone(),
    });
    content.insert("actions".into(), actions.unwrap_or(Value::Null));
    content
}

fn hash(content: serde_json::Map<String, Value>) -> String {
    let canonical = python_json::dumps_sorted(&Value::Object(content));
    Sha256::digest(canonical.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/fingerprint.json")).unwrap()
    }

    #[test]
    fn fingerprint_matches_upstreams_hash_exactly() {
        let fixture = fixture();
        let ours = fingerprint(&fixture["page"]);
        assert_eq!(ours, fixture["upstream"].as_str().unwrap());
        assert_eq!(ours, fixture["sha256"].as_str().unwrap());
    }

    #[test]
    fn fingerprint_canonical_form_matches_upstream() {
        let fixture = fixture();
        let mut content = serde_json::Map::new();
        for key in ["url", "text", "actions", "scroll"] {
            content.insert(key.into(), fixture["page"][key].clone());
        }
        assert_eq!(
            python_json::dumps_sorted(&Value::Object(content)),
            fixture["canonical"].as_str().unwrap()
        );
    }

    #[test]
    fn fingerprint_ignores_fields_upstream_excludes() {
        let fixture = fixture();
        let mut page = fixture["page"].clone();
        let before = fingerprint(&page);
        page["title"] = json!("a different title");
        page["guards"] = json!({});
        page["marker"] = json!([1, 2, 3]);
        assert_eq!(fingerprint(&page), before);

        page["scroll"] = json!({"y": 400, "height": 2400});
        assert_ne!(fingerprint(&page), before);
    }

    #[test]
    fn fingerprint_ignores_offscreen_controls_and_context() {
        let fixture = fixture();
        let mut page = fixture["page"].clone();
        let before = fingerprint(&page);
        let actions = page["actions"].as_array_mut().unwrap();
        actions.push(
            json!({"id": "e99", "kind": "click", "label": "Sale ends in 59s",
            "offscreen": true, "rect": {"x": 0, "y": 2400, "w": 90, "h": 20}}),
        );
        actions[0]["context"] = json!("Checkout");
        actions[0]["section"] = json!("Your order");
        assert_eq!(fingerprint(&page), before);

        page["actions"][0]["label"] = json!("renamed");
        assert_ne!(fingerprint(&page), before);
    }

    /// A rotating ad frame's text never counts as progress; a frame that
    /// opens (a booking widget after a click) does.
    #[test]
    fn frames_count_by_origin_not_by_text() {
        let page = fixture()["page"].clone();
        let frame = |origin: &str, text: &str| JevFrameText {
            origin: origin.into(),
            text: text.into(),
        };
        assert_eq!(fingerprint_with_frames(&page, &[]), fingerprint(&page));
        let ad = fingerprint_with_frames(&page, &[frame("https://ads.test", "Buy shoes")]);
        assert_eq!(
            fingerprint_with_frames(&page, &[frame("https://ads.test", "Cheap flights")]),
            ad
        );
        assert_ne!(ad, fingerprint(&page));
        assert_ne!(
            fingerprint_with_frames(
                &page,
                &[
                    frame("https://ads.test", "Buy shoes"),
                    frame("https://widgets.test", "Complete your reservation"),
                ]
            ),
            ad
        );
    }
}
