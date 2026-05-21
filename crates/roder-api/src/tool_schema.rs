use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolSchemaMode {
    Strict,
    Warning,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSchemaPolicy {
    pub mode: ToolSchemaMode,
}

impl ToolSchemaPolicy {
    pub fn strict() -> Self {
        Self {
            mode: ToolSchemaMode::Strict,
        }
    }

    pub fn warning() -> Self {
        Self {
            mode: ToolSchemaMode::Warning,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolSchemaLintKind {
    NestedRequiredArray,
    MissingAdditionalProperties,
    AmbiguousFieldName,
    MismatchedDefault,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSchemaLint {
    pub tool_name: String,
    pub pointer: String,
    pub kind: ToolSchemaLintKind,
    pub message: String,
    pub severity: ToolSchemaMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSchemaReport {
    pub tool_name: String,
    pub schema: Value,
    #[serde(default)]
    pub lints: Vec<ToolSchemaLint>,
}

pub fn normalize_tool_schema(
    tool_name: &str,
    schema: &Value,
    policy: ToolSchemaPolicy,
) -> ToolSchemaReport {
    let schema = normalize_value(schema);
    let mut lints = Vec::new();
    lint_value(tool_name, "", &schema, policy, &mut lints);
    ToolSchemaReport {
        tool_name: tool_name.to_string(),
        schema,
        lints,
    }
}

fn normalize_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut normalized = Map::new();
            push_key("type", object, &mut normalized);
            push_key("required", object, &mut normalized);
            if let Some(properties) = object.get("properties") {
                normalized.insert("properties".to_string(), normalize_properties(properties));
            }
            push_key("additionalProperties", object, &mut normalized);
            let mut rest = object
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "type" | "required" | "properties" | "additionalProperties"
                    )
                })
                .collect::<Vec<_>>();
            rest.sort_by(|(left, _), (right, _)| left.cmp(right));
            for (key, value) in rest {
                normalized.insert(key.clone(), normalize_value(value));
            }
            Value::Object(normalized)
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize_value).collect()),
        _ => value.clone(),
    }
}

fn normalize_properties(value: &Value) -> Value {
    let Value::Object(properties) = value else {
        return normalize_value(value);
    };
    let mut normalized = Map::new();
    let mut entries = properties.iter().collect::<Vec<_>>();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (key, value) in entries {
        normalized.insert(key.clone(), normalize_value(value));
    }
    Value::Object(normalized)
}

fn push_key(key: &str, source: &Map<String, Value>, target: &mut Map<String, Value>) {
    if let Some(value) = source.get(key) {
        target.insert(key.to_string(), normalize_value(value));
    }
}

fn lint_value(
    tool_name: &str,
    pointer: &str,
    value: &Value,
    policy: ToolSchemaPolicy,
    lints: &mut Vec<ToolSchemaLint>,
) {
    let Value::Object(object) = value else {
        return;
    };
    if object.get("type").and_then(Value::as_str) == Some("object")
        && object.get("additionalProperties").and_then(Value::as_bool) != Some(false)
    {
        push_lint(
            lints,
            tool_name,
            pointer,
            ToolSchemaLintKind::MissingAdditionalProperties,
            "object schema should set additionalProperties: false",
            policy,
        );
    }
    if pointer != "" && object.get("required").is_some_and(Value::is_array) {
        push_lint(
            lints,
            tool_name,
            pointer,
            ToolSchemaLintKind::NestedRequiredArray,
            "nested object schemas with required arrays are brittle for model tool calls",
            policy,
        );
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            if matches!(name.as_str(), "file" | "text" | "input" | "value") {
                push_lint(
                    lints,
                    tool_name,
                    &format!("{pointer}/properties/{name}"),
                    ToolSchemaLintKind::AmbiguousFieldName,
                    "prefer specific coding-agent argument names such as path, content, query, or command",
                    policy,
                );
            }
            if let Some(default) = property.get("default")
                && default.is_null()
            {
                push_lint(
                    lints,
                    tool_name,
                    &format!("{pointer}/properties/{name}/default"),
                    ToolSchemaLintKind::MismatchedDefault,
                    "null defaults are ambiguous unless the runtime applies the same default",
                    policy,
                );
            }
            lint_value(
                tool_name,
                &format!("{pointer}/properties/{name}"),
                property,
                policy,
                lints,
            );
        }
    }
}

fn push_lint(
    lints: &mut Vec<ToolSchemaLint>,
    tool_name: &str,
    pointer: &str,
    kind: ToolSchemaLintKind,
    message: &str,
    policy: ToolSchemaPolicy,
) {
    lints.push(ToolSchemaLint {
        tool_name: tool_name.to_string(),
        pointer: if pointer.is_empty() {
            "/".to_string()
        } else {
            pointer.to_string()
        },
        kind,
        message: message.to_string(),
        severity: policy.mode,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schema_normalizes_required_before_properties_at_every_object_layer() {
        let schema = serde_json::json!({
            "additionalProperties": false,
            "properties": {
                "edits": {
                    "properties": {
                        "new_string": { "type": "string" },
                        "old_string": { "type": "string" }
                    },
                    "additionalProperties": false,
                    "required": ["old_string", "new_string"],
                    "type": "object"
                },
                "path": { "type": "string" }
            },
            "required": ["path", "edits"],
            "type": "object"
        });

        let report = normalize_tool_schema("multi_edit", &schema, ToolSchemaPolicy::strict());
        let json = serde_json::to_string(&report.schema).unwrap();
        let root_required = json.find(r#""required""#).unwrap();
        let root_properties = json.find(r#""properties""#).unwrap();
        let nested = json.find(r#""edits":{"#).unwrap();
        let nested_required = json[nested..].find(r#""required""#).unwrap() + nested;
        let nested_properties = json[nested..].find(r#""properties""#).unwrap() + nested;

        assert!(root_required < root_properties, "{json}");
        assert!(nested_required < nested_properties, "{json}");
    }

    #[test]
    fn tool_schema_lints_include_tool_name_and_json_pointer() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "object",
                    "required": ["value"],
                    "properties": {
                        "value": { "type": "string", "default": null }
                    }
                }
            }
        });

        let report = normalize_tool_schema("bad_tool", &schema, ToolSchemaPolicy::strict());

        assert!(report.lints.iter().any(|lint| lint.tool_name == "bad_tool"
            && lint.pointer == "/"
            && lint.kind == ToolSchemaLintKind::MissingAdditionalProperties));
        assert!(
            report
                .lints
                .iter()
                .any(|lint| lint.pointer == "/properties/input"
                    && lint.kind == ToolSchemaLintKind::NestedRequiredArray)
        );
    }
}
