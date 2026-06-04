use roder_edit_core::{EditOptions, TextEdit};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn roder_edit_tools_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[wasm_bindgen]
pub fn format_line_numbered_read_json(input_json: &str) -> String {
    json_result(|| {
        let request: ReadJsonRequest =
            serde_json::from_str(input_json).map_err(JsonFailure::from)?;
        let text = roder_edit_core::format_line_numbered_read(
            &request.text,
            roder_edit_core::ReadFormatOptions {
                start_line: request.start_line.unwrap_or(1),
                limit: request.limit.unwrap_or(200),
            },
        );
        Ok(ReadJsonResponse { text })
    })
}

#[wasm_bindgen]
pub fn apply_edit_json(input_json: &str) -> String {
    json_result(|| {
        let request: EditJsonRequest =
            serde_json::from_str(input_json).map_err(JsonFailure::from)?;
        let (content, result) = roder_edit_core::apply_edit(
            request.path,
            &request.content,
            &request.old_string,
            &request.new_string,
            request.options.unwrap_or_default(),
        )
        .map_err(|err| JsonFailure::from(serde_json::json!({ "error": edit_error_json(err) })))?;
        Ok(EditJsonResponse { content, result })
    })
}

#[wasm_bindgen]
pub fn apply_multi_edit_json(input_json: &str) -> String {
    json_result(|| {
        let request: MultiEditJsonRequest =
            serde_json::from_str(input_json).map_err(JsonFailure::from)?;
        let (content, result) = roder_edit_core::apply_multi_edit(
            request.path,
            &request.content,
            &request.edits,
            request.options.unwrap_or_default(),
        )
        .map_err(|err| JsonFailure::from(serde_json::json!({ "error": edit_error_json(err) })))?;
        Ok(EditJsonResponse { content, result })
    })
}

#[wasm_bindgen]
pub fn codex_patch_hunks_json(patch: &str) -> String {
    json_result(|| roder_edit_core::patch::codex_patch_hunks(patch).map_err(JsonFailure::from))
}

fn json_result<T, F>(f: F) -> String
where
    T: Serialize,
    F: FnOnce() -> Result<T, JsonFailure>,
{
    match f() {
        Ok(value) => serde_json::json!({ "ok": true, "value": value }).to_string(),
        Err(err) => serde_json::json!({ "ok": false, "error": err }).to_string(),
    }
}

#[derive(Debug, Serialize)]
struct JsonFailure {
    kind: String,
    message: String,
    data: serde_json::Value,
}

impl From<serde_json::Error> for JsonFailure {
    fn from(err: serde_json::Error) -> Self {
        Self {
            kind: "invalid_json".to_string(),
            message: err.to_string(),
            data: serde_json::Value::Null,
        }
    }
}

impl From<anyhow::Error> for JsonFailure {
    fn from(err: anyhow::Error) -> Self {
        Self {
            kind: "operation_failed".to_string(),
            message: err.to_string(),
            data: serde_json::Value::Null,
        }
    }
}

impl From<serde_json::Value> for JsonFailure {
    fn from(value: serde_json::Value) -> Self {
        Self {
            kind: value
                .pointer("/error/kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("edit_failed")
                .to_string(),
            message: value.to_string(),
            data: value,
        }
    }
}

fn edit_error_json(err: roder_edit_core::EditApplyError) -> serde_json::Value {
    match err {
        roder_edit_core::EditApplyError::OldStringNotFound { edit, candidates } => {
            serde_json::json!({
                "kind": "old_string_not_found",
                "edit": edit,
                "candidates": candidates,
            })
        }
        roder_edit_core::EditApplyError::OldStringAmbiguous {
            edit,
            occurrences,
            candidates,
        } => serde_json::json!({
            "kind": "old_string_ambiguous",
            "edit": edit,
            "occurrences": occurrences,
            "candidates": candidates,
        }),
    }
}

#[derive(Debug, Deserialize)]
struct ReadJsonRequest {
    text: String,
    start_line: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct ReadJsonResponse {
    text: String,
}

#[derive(Debug, Deserialize)]
struct EditJsonRequest {
    path: String,
    content: String,
    old_string: String,
    new_string: String,
    options: Option<EditOptions>,
}

#[derive(Debug, Deserialize)]
struct MultiEditJsonRequest {
    path: String,
    content: String,
    edits: Vec<TextEdit>,
    options: Option<EditOptions>,
}

#[derive(Debug, Serialize)]
struct EditJsonResponse {
    content: String,
    result: roder_edit_core::EditToolResult,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn edit_binding_returns_json_success() {
        let output = apply_edit_json(
            r#"{"path":"a.txt","content":"old","old_string":"old","new_string":"new"}"#,
        );
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["value"]["content"], "new");
    }
}
