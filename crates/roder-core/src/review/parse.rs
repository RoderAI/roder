//! Recovers a [`ReviewOutput`] from the reviewer's final message.
//!
//! Roder has no structured-output plumbing, so the rubric asks for a bare JSON
//! object and this module puts the response back together with a four-tier
//! ladder:
//!
//! 1. the whole message parsed as JSON,
//! 2. the contents of the first fenced code block,
//! 3. the first balanced `{…}` substring,
//! 4. the raw text carried in `overallExplanation`.
//!
//! Tiers 1–3 additionally normalize the decoded value, because models routinely
//! emit the snake_case spelling of the schema or a numeric priority.

use roder_api::review::ReviewOutput;
use serde_json::{Map, Value};

/// Parses the reviewer's final message. Never fails: an unparseable message
/// becomes a finding-less output whose explanation is the raw text.
pub fn parse_review_output(text: &str) -> ReviewOutput {
    for candidate in candidates(text) {
        if let Some(output) = decode(&candidate) {
            return output;
        }
    }
    let text = text.trim();
    ReviewOutput {
        overall_explanation: (!text.is_empty()).then(|| text.to_string()),
        ..ReviewOutput::default()
    }
}

/// Tiers 1–3, in order.
fn candidates(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    let mut candidates = vec![trimmed.to_string()];
    candidates.extend(fenced_blocks(trimmed));
    candidates.extend(balanced_objects(trimmed));
    candidates
}

fn decode(candidate: &str) -> Option<ReviewOutput> {
    let mut value = serde_json::from_str::<Value>(candidate).ok()?;
    // A bare array or string parses fine but is not a review output; requiring
    // an object keeps tier 3 from matching a stray JSON literal in the prose.
    if !value.is_object() {
        return None;
    }
    normalize(&mut value);
    serde_json::from_value::<ReviewOutput>(value).ok()
}

/// Contents of every ```-fenced block, outermost first.
fn fenced_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("```") {
        let after_open = &rest[open + 3..];
        // Skip the info string (`json`, `JSON`, …) on the opening fence.
        let body_start = match after_open.find('\n') {
            Some(newline) => newline + 1,
            None => break,
        };
        let body = &after_open[body_start..];
        match body.find("```") {
            Some(close) => {
                blocks.push(body[..close].trim().to_string());
                rest = &body[close + 3..];
            }
            None => {
                blocks.push(body.trim().to_string());
                break;
            }
        }
    }
    blocks
}

/// Every balanced `{…}` substring starting at a top-level `{`, outermost first.
/// String literals and their escapes are respected so a `}` inside a body does
/// not truncate the object.
fn balanced_objects(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut objects = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'{' {
            index += 1;
            continue;
        }
        match balanced_end(bytes, index) {
            Some(end) => {
                objects.push(text[index..=end].to_string());
                index = end + 1;
            }
            None => break,
        }
    }
    objects
}

fn balanced_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, byte) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Snake_case spellings of the schema keys, mapped to the camelCase names the
/// `roder-api` types deserialize from.
const KEY_ALIASES: &[(&str, &str)] = &[
    ("confidence_score", "confidenceScore"),
    ("code_location", "codeLocation"),
    ("absolute_file_path", "absoluteFilePath"),
    ("line_range", "lineRange"),
    ("overall_correctness", "overallCorrectness"),
    ("overall_explanation", "overallExplanation"),
    ("overall_confidence_score", "overallConfidenceScore"),
];

fn normalize(value: &mut Value) {
    match value {
        Value::Object(object) => {
            rename_keys(object);
            match object.get("priority").and_then(normalized_priority) {
                Some(priority) => {
                    object.insert("priority".to_string(), Value::String(priority));
                }
                // `ReviewPriority` is not an `Option`, so an unrecognized value
                // has to be removed rather than nulled for `#[serde(default)]`
                // to supply P2.
                None => {
                    object.remove("priority");
                }
            }
            for entry in object.values_mut() {
                normalize(entry);
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize(item);
            }
        }
        _ => {}
    }
}

fn rename_keys(object: &mut Map<String, Value>) {
    for (from, to) in KEY_ALIASES {
        if object.contains_key(*to) {
            continue;
        }
        if let Some(value) = object.remove(*from) {
            object.insert((*to).to_string(), value);
        }
    }
}

/// Accepts the codex-style numeric priority and any casing of `p0`..`p3`;
/// anything else yields `None` so the field falls back to its default.
fn normalized_priority(priority: &Value) -> Option<String> {
    let level = match priority {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_f64().map(|level| level as u64))?,
        Value::String(text) => text
            .trim()
            .strip_prefix(['p', 'P'])
            .and_then(|level| level.parse::<u64>().ok())?,
        _ => return None,
    };
    (level <= 3).then(|| format!("p{level}"))
}

#[cfg(test)]
mod tests {
    use roder_api::review::ReviewPriority;

    use super::*;

    const FINDING: &str = r#"{
      "findings": [
        {
          "title": "[P1] Off-by-one in the loop bound",
          "body": "The loop overruns the slice.",
          "confidenceScore": 0.8,
          "priority": "p1",
          "codeLocation": {
            "absoluteFilePath": "/repo/src/lib.rs",
            "lineRange": { "start": 10, "end": 12 }
          }
        }
      ],
      "overallCorrectness": "patch is incorrect",
      "overallExplanation": "One blocking bug.",
      "overallConfidenceScore": 0.7
    }"#;

    #[test]
    fn tier_one_parses_a_bare_json_object() {
        let output = parse_review_output(FINDING);
        assert_eq!(output.findings.len(), 1);
        assert_eq!(output.findings[0].priority, ReviewPriority::P1);
        assert_eq!(
            output.findings[0].code_location.absolute_file_path,
            std::path::PathBuf::from("/repo/src/lib.rs")
        );
        assert_eq!(
            output.overall_correctness.as_deref(),
            Some("patch is incorrect")
        );
    }

    #[test]
    fn tier_two_parses_a_fenced_block() {
        let text = format!("Here is the review.\n\n```json\n{FINDING}\n```\n");
        let output = parse_review_output(&text);
        assert_eq!(output.findings.len(), 1);
        assert_eq!(output.overall_confidence_score, Some(0.7));
    }

    #[test]
    fn tier_three_parses_an_embedded_object_containing_braces() {
        let text = format!(
            "Sure — findings below.\n{}\nLet me know if you want a patch.",
            FINDING.replace(
                "The loop overruns the slice.",
                "Use `HashMap {} literals` carefully }}"
            )
        );
        let output = parse_review_output(&text);
        assert_eq!(output.findings.len(), 1);
        assert!(output.findings[0].body.contains("HashMap"));
    }

    #[test]
    fn tier_four_carries_the_raw_text() {
        let output = parse_review_output("I could not find anything worth flagging.");
        assert!(output.findings.is_empty());
        assert_eq!(
            output.overall_explanation.as_deref(),
            Some("I could not find anything worth flagging.")
        );
    }

    #[test]
    fn tier_four_leaves_an_empty_message_empty() {
        let output = parse_review_output("   \n ");
        assert!(output.findings.is_empty());
        assert!(output.overall_explanation.is_none());
    }

    #[test]
    fn snake_case_keys_and_numeric_priority_are_normalized() {
        let text = r#"{
          "findings": [
            {
              "title": "Missing bounds check",
              "body": "…",
              "confidence_score": 0.5,
              "priority": 0,
              "code_location": {
                "absolute_file_path": "/repo/src/main.rs",
                "line_range": { "start": 3, "end": 3 }
              }
            }
          ],
          "overall_correctness": "patch is incorrect",
          "overall_explanation": "Blocking."
        }"#;
        let output = parse_review_output(text);
        assert_eq!(output.findings.len(), 1);
        assert_eq!(output.findings[0].priority, ReviewPriority::P0);
        assert_eq!(output.findings[0].confidence_score, Some(0.5));
        assert_eq!(output.overall_explanation.as_deref(), Some("Blocking."));
    }

    #[test]
    fn unknown_priority_falls_back_to_the_default() {
        let text = r#"{"findings":[{"title":"t","body":"b","priority":"urgent",
          "codeLocation":{"absoluteFilePath":"/a","lineRange":{"start":1,"end":1}}}]}"#;
        let output = parse_review_output(text);
        assert_eq!(output.findings[0].priority, ReviewPriority::P2);
    }

    #[test]
    fn a_json_array_is_not_mistaken_for_a_review() {
        let output = parse_review_output("[1, 2, 3]");
        assert!(output.findings.is_empty());
        assert_eq!(output.overall_explanation.as_deref(), Some("[1, 2, 3]"));
    }
}
