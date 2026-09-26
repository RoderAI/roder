//! Python's `json.dumps` output, reproduced exactly.
//!
//! Upstream Jev fingerprints an observation with
//! `sha256(json.dumps(content, sort_keys=True))`. Matching that hash means
//! matching CPython's default serialization: sorted keys, `", "` and `": "`
//! separators, and non-ASCII escaped. Only the fingerprint needs this; request
//! bodies use compact separators and go through `serde_json`.

use std::fmt::Write as _;

use serde_json::Value;

/// `json.dumps(value, sort_keys=True)`.
pub(crate) fn dumps_sorted(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, true);
    out
}

/// `json.dumps(value)`, keeping insertion order. Upstream uses this for the
/// text helper's user message.
pub(crate) fn dumps(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, false);
    out
}

fn write_value(out: &mut String, value: &Value, sort_keys: bool) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => {
            // CPython writes ints bare and floats with `repr`, which is the
            // shortest round-tripping form that serde_json also produces.
            let _ = write!(out, "{number}");
        }
        Value::String(text) => write_string(out, text),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_value(out, item, sort_keys);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            if sort_keys {
                // Python compares str by code point; Rust's byte order over
                // UTF-8 is the same ordering.
                keys.sort_unstable();
            }
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_string(out, key);
                out.push_str(": ");
                write_value(out, &map[key], sort_keys);
            }
            out.push('}');
        }
    }
}

/// CPython's `py_encode_basestring_ascii`: escape the JSON specials, the C0
/// controls, and everything outside ASCII as `\uXXXX` (surrogate pairs above
/// the BMP).
fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            character if !(' '..='\u{7e}').contains(&character) => {
                let code = character as u32;
                if code > 0xffff {
                    let value = code - 0x1_0000;
                    let _ = write!(
                        out,
                        "\\u{:04x}\\u{:04x}",
                        0xd800 + (value >> 10),
                        0xdc00 + (value & 0x3ff)
                    );
                } else {
                    let _ = write!(out, "\\u{code:04x}");
                }
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_sorted_and_separators_carry_spaces() {
        let value = json!({"b": 1, "a": [1, 2], "c": {"z": true, "y": null}});
        assert_eq!(
            dumps_sorted(&value),
            r#"{"a": [1, 2], "b": 1, "c": {"y": null, "z": true}}"#
        );
    }

    #[test]
    fn non_ascii_is_escaped_like_cpython() {
        assert_eq!(
            dumps_sorted(&json!("Cabin → Economy")),
            r#""Cabin \u2192 Economy""#
        );
        assert_eq!(dumps_sorted(&json!("£81")), r#""\u00a381""#);
        assert_eq!(dumps_sorted(&json!("emoji 🙂")), r#""emoji \ud83d\ude42""#);
    }

    #[test]
    fn control_characters_use_the_short_escapes() {
        assert_eq!(dumps_sorted(&json!("a\nb\tc\rd")), r#""a\nb\tc\rd""#);
        assert_eq!(dumps_sorted(&json!("bell\u{7}")), r#""bell\u0007""#);
        assert_eq!(
            dumps_sorted(&json!("quote\"slash\\")),
            r#""quote\"slash\\""#
        );
    }

    #[test]
    fn numbers_keep_integer_and_float_forms() {
        assert_eq!(
            dumps_sorted(&json!({"i": 560, "f": 24.0, "n": -0.5})),
            r#"{"f": 24.0, "i": 560, "n": -0.5}"#
        );
    }

    #[test]
    fn insertion_order_is_kept_when_keys_are_not_sorted() {
        let value = json!({"goal": 1, "field": 2, "page": 3});
        assert_eq!(dumps(&value), r#"{"goal": 1, "field": 2, "page": 3}"#);
        assert_eq!(
            dumps_sorted(&value),
            r#"{"field": 2, "goal": 1, "page": 3}"#
        );
    }

    #[test]
    fn empty_containers_match() {
        assert_eq!(
            dumps_sorted(&json!({"a": {}, "b": []})),
            r#"{"a": {}, "b": []}"#
        );
    }
}
