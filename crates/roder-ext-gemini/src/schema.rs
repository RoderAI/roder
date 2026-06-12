use serde_json::Value;

pub(crate) fn gemini_schema(mut schema: Value) -> Value {
    strip_unsupported_schema_fields(&mut schema);
    schema
}

/// Gemini's `generateContent` accepts a strict OpenAPI-style proto subset:
/// JSON-schema keywords like `const`, `examples`, `$schema`, and union
/// `type` arrays are rejected with HTTP 400 for the whole request, so they
/// must be stripped or rewritten before the call.
fn strip_unsupported_schema_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("additionalProperties");
            object.remove("examples");
            object.remove("$schema");
            object.retain(|key, _| !key.starts_with("x-"));
            // `const: V` -> `enum: [V]` (proto has no const field).
            if let Some(const_value) = object.remove("const")
                && !object.contains_key("enum")
            {
                object.insert("enum".to_string(), Value::Array(vec![const_value]));
            }
            // `type: ["string", "null"]` -> `type: "string", nullable: true`.
            if let Some(Value::Array(types)) = object.get("type") {
                let nullable = types.iter().any(|t| t.as_str() == Some("null"));
                let first = types
                    .iter()
                    .find(|t| t.as_str() != Some("null"))
                    .cloned()
                    .unwrap_or(Value::String("string".to_string()));
                object.insert("type".to_string(), first);
                if nullable {
                    object.insert("nullable".to_string(), Value::Bool(true));
                }
            }
            for child in object.values_mut() {
                strip_unsupported_schema_fields(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_unsupported_schema_fields(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn removes_additional_properties_recursively() {
        let schema = gemini_schema(json!({
            "type": "object",
            "additionalProperties": false,
            "x-roder": { "display": "internal" },
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "x-roder": { "display": "internal" },
                        "properties": {
                            "name": { "type": "string" }
                        }
                    }
                }
            }
        }));

        assert!(schema.get("additionalProperties").is_none());
        assert!(schema.get("x-roder").is_none());
        assert!(
            schema
                .pointer("/properties/items/items/additionalProperties")
                .is_none()
        );
        assert!(schema.pointer("/properties/items/items/x-roder").is_none());
        assert_eq!(
            schema.pointer("/properties/items/items/properties/name/type"),
            Some(&json!("string"))
        );
    }

    #[test]
    fn rewrites_const_examples_and_union_types() {
        let schema = gemini_schema(json!({
            "type": "object",
            "$schema": "http://json-schema.org/draft-07/schema#",
            "properties": {
                "kind": { "const": "deploy", "examples": ["deploy"] },
                "name": { "type": ["string", "null"] },
                "steps": {
                    "type": "array",
                    "items": {
                        "oneOf": [
                            { "type": "object", "properties": { "op": { "const": "create" } } }
                        ]
                    }
                }
            }
        }));

        assert!(schema.get("$schema").is_none());
        assert_eq!(
            schema.pointer("/properties/kind/enum"),
            Some(&json!(["deploy"]))
        );
        assert!(schema.pointer("/properties/kind/const").is_none());
        assert!(schema.pointer("/properties/kind/examples").is_none());
        assert_eq!(
            schema.pointer("/properties/name/type"),
            Some(&json!("string"))
        );
        assert_eq!(
            schema.pointer("/properties/name/nullable"),
            Some(&json!(true))
        );
        assert_eq!(
            schema.pointer("/properties/steps/items/oneOf/0/properties/op/enum"),
            Some(&json!(["create"]))
        );
    }
}
