use serde_json::Value;

pub(crate) fn gemini_schema(mut schema: Value) -> Value {
    strip_unsupported_schema_fields(&mut schema);
    schema
}

fn strip_unsupported_schema_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("additionalProperties");
            object.retain(|key, _| !key.starts_with("x-"));
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
        assert!(
            schema
                .pointer("/properties/items/items/x-roder")
                .is_none()
        );
        assert_eq!(
            schema.pointer("/properties/items/items/properties/name/type"),
            Some(&json!("string"))
        );
    }
}
