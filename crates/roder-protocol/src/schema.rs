use serde_json::{Value, json};

use crate::methods::{AppServerMethodManifest, app_server_method_manifest};

pub fn app_server_manifest_json() -> Value {
    serde_json::to_value(app_server_method_manifest()).expect("manifest serializes")
}

pub fn app_server_json_schema() -> Value {
    let manifest = app_server_method_manifest();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://roder.dev/schemas/app-server/roder-app-server.v1.json",
        "title": "Roder App Server Manifest",
        "type": "object",
        "required": ["schemaVersion", "unknownMethodsAllowed", "methods"],
        "properties": {
            "schemaVersion": { "const": manifest.schema_version },
            "unknownMethodsAllowed": { "type": "boolean" },
            "methods": {
                "type": "array",
                "items": method_spec_schema(&manifest),
            },
        },
        "additionalProperties": false,
    })
}

fn method_spec_schema(manifest: &AppServerMethodManifest) -> Value {
    let methods = manifest
        .methods
        .iter()
        .map(|spec| spec.method)
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "required": [
            "method",
            "paramsType",
            "resultType",
            "stability",
            "featureGroup",
            "idempotency",
            "sideEffect",
        ],
        "properties": {
            "method": { "type": "string", "enum": methods },
            "paramsType": { "type": "string" },
            "resultType": { "type": "string" },
            "stability": { "enum": ["stable", "experimental"] },
            "featureGroup": { "type": "string" },
            "idempotency": { "enum": ["idempotent", "nonIdempotent"] },
            "sideEffect": { "enum": ["readOnly", "localState", "externalProcess"] },
            "notifications": {
                "type": "array",
                "items": { "type": "string" },
            },
        },
        "additionalProperties": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_contains_manifest_version_and_method_enum() {
        let schema = app_server_json_schema();
        assert_eq!(schema["properties"]["schemaVersion"]["const"], 1);
        let method_enum = schema["properties"]["methods"]["items"]["properties"]["method"]["enum"]
            .as_array()
            .expect("method enum");
        assert!(method_enum.iter().any(|value| value == "thread/start"));
        assert!(method_enum.iter().any(|value| value == "memory/query"));
    }

    #[test]
    fn manifest_json_allows_unknown_methods_for_raw_clients() {
        let manifest = app_server_manifest_json();
        assert_eq!(manifest["unknownMethodsAllowed"], true);
        assert!(manifest["methods"].as_array().unwrap().len() > 100);
    }

    #[test]
    fn checked_app_server_schema_file_matches_generator() {
        let checked: Value = serde_json::from_str(include_str!(
            "../../../schemas/app-server/methods.schema.json"
        ))
        .expect("checked schema json");
        assert_eq!(checked, app_server_json_schema());
    }

    #[test]
    fn checked_app_server_manifest_file_matches_generator() {
        let checked: Value = serde_json::from_str(include_str!(
            "../../../schemas/app-server/roder-app-server.v1.json"
        ))
        .expect("checked manifest json");
        assert_eq!(checked, app_server_manifest_json());
    }
}
