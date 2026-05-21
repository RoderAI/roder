use serde_json::Value;

const REDACTED: &str = "<redacted>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedactionRule {
    SensitiveKey(String),
    JsonPointer(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedactionSummary {
    pub redacted_fields: Vec<String>,
}

impl RedactionSummary {
    pub fn redacted_count(&self) -> usize {
        self.redacted_fields.len()
    }
}

#[derive(Debug, Clone)]
pub struct TranscriptRedactor {
    rules: Vec<RedactionRule>,
}

impl Default for TranscriptRedactor {
    fn default() -> Self {
        Self::new(default_rules())
    }
}

impl TranscriptRedactor {
    pub fn new(rules: Vec<RedactionRule>) -> Self {
        Self { rules }
    }

    pub fn redact_value(&self, value: &mut Value) -> RedactionSummary {
        let mut summary = RedactionSummary::default();
        redact_sensitive_keys(value, "$", &mut summary);
        for rule in &self.rules {
            match rule {
                RedactionRule::SensitiveKey(key) => {
                    redact_named_key(value, "$", key, &mut summary);
                }
                RedactionRule::JsonPointer(pointer) => {
                    redact_pointer(value, pointer, &mut summary);
                }
            }
        }
        summary.redacted_fields.sort();
        summary.redacted_fields.dedup();
        summary
    }
}

pub fn default_rules() -> Vec<RedactionRule> {
    [
        "authorization",
        "api_key",
        "apikey",
        "access_token",
        "refresh_token",
        "id_token",
        "token",
        "secret",
        "password",
        "bearer",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GEMINI_API_KEY",
        "XAI_API_KEY",
        "SUPERGROK_API_KEY",
    ]
    .into_iter()
    .map(|key| RedactionRule::SensitiveKey(key.to_string()))
    .collect()
}

fn redact_sensitive_keys(value: &mut Value, path: &str, summary: &mut RedactionSummary) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                let child_path = format!("{path}/{}", escape_pointer_segment(key));
                if is_sensitive_key(key) {
                    redact_leaf(child, child_path, summary);
                } else if looks_like_bearer_header(key, child) {
                    *child = Value::String("Bearer <redacted>".to_string());
                    summary.redacted_fields.push(child_path);
                } else {
                    redact_sensitive_keys(child, &child_path, summary);
                }
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter_mut().enumerate() {
                redact_sensitive_keys(child, &format!("{path}/{index}"), summary);
            }
        }
        _ => {}
    }
}

fn redact_named_key(value: &mut Value, path: &str, target: &str, summary: &mut RedactionSummary) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                let child_path = format!("{path}/{}", escape_pointer_segment(key));
                if normalize_key(key) == normalize_key(target) {
                    redact_leaf(child, child_path, summary);
                } else {
                    redact_named_key(child, &child_path, target, summary);
                }
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter_mut().enumerate() {
                redact_named_key(child, &format!("{path}/{index}"), target, summary);
            }
        }
        _ => {}
    }
}

fn redact_pointer(value: &mut Value, pointer: &str, summary: &mut RedactionSummary) {
    if pointer.is_empty() {
        *value = Value::String(REDACTED.to_string());
        summary.redacted_fields.push("$".to_string());
        return;
    }
    if let Some(target) = value.pointer_mut(pointer) {
        redact_leaf(target, format!("${pointer}"), summary);
    }
}

fn redact_leaf(value: &mut Value, path: String, summary: &mut RedactionSummary) {
    if !value.is_null() {
        *value = Value::String(REDACTED.to_string());
        summary.redacted_fields.push(path);
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = normalize_key(key);
    default_sensitive_keys()
        .iter()
        .any(|candidate| normalized == *candidate || normalized.ends_with(candidate))
}

fn looks_like_bearer_header(key: &str, value: &Value) -> bool {
    key.eq_ignore_ascii_case("authorization")
        && value
            .as_str()
            .is_some_and(|text| text.to_ascii_lowercase().starts_with("bearer "))
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn default_sensitive_keys() -> &'static [&'static str] {
    &[
        "authorization",
        "apikey",
        "accesstoken",
        "refreshtoken",
        "idtoken",
        "token",
        "secret",
        "password",
        "openaikey",
        "anthropicapikey",
        "geminiapikey",
        "xaiapikey",
        "supergrokapikey",
    ]
}

fn escape_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn redaction_removes_auth_tokens_api_keys_and_bearer_headers() {
        let mut value = json!({
            "headers": {
                "authorization": "Bearer raw-token",
                "x-safe": "keep"
            },
            "env": {
                "OPENAI_API_KEY": "sk-raw",
                "PATH": "/usr/bin"
            },
            "nested": {
                "refreshToken": "refresh-raw",
                "password": "pw-raw"
            }
        });

        let summary = TranscriptRedactor::default().redact_value(&mut value);

        assert_eq!(value["headers"]["authorization"], REDACTED);
        assert_eq!(value["env"]["OPENAI_API_KEY"], REDACTED);
        assert_eq!(value["nested"]["refreshToken"], REDACTED);
        assert_eq!(value["nested"]["password"], REDACTED);
        assert_eq!(value["headers"]["x-safe"], "keep");
        let serialized = serde_json::to_string(&value).unwrap();
        assert!(!serialized.contains("raw-token"));
        assert!(!serialized.contains("sk-raw"));
        assert_eq!(summary.redacted_count(), 4);
    }

    #[test]
    fn redaction_applies_configured_json_pointer_paths() {
        let mut value = json!({
            "request": {
                "params": {
                    "command": "deploy --token raw-command-token"
                }
            }
        });
        let redactor = TranscriptRedactor::new(vec![RedactionRule::JsonPointer(
            "/request/params/command".to_string(),
        )]);

        let summary = redactor.redact_value(&mut value);

        assert_eq!(value["request"]["params"]["command"], REDACTED);
        assert_eq!(
            summary.redacted_fields,
            vec!["$/request/params/command".to_string()]
        );
        assert!(
            !serde_json::to_string(&value)
                .unwrap()
                .contains("raw-command-token")
        );
    }
}
