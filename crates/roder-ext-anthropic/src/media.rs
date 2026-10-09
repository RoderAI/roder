use roder_api::transcript::{ToolResultRecord, tool_result_image};
use serde_json::{Value, json};

pub(crate) fn tool_result_content(result: &ToolResultRecord) -> Value {
    let Some((mime, data)) = tool_result_image(result.display_payload.as_ref()) else {
        return json!(result.result);
    };
    json!([
        {"type":"text","text":result.result},
        {"type":"image","source":{"type":"base64","media_type":mime,"data":data}}
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_images_are_content_blocks_including_failed_actions() {
        let result = ToolResultRecord {
            id: "call".into(),
            name: Some("cua_click".into()),
            result: "refused".into(),
            is_error: true,
            display_payload: Some(
                json!({"__view_image":{"image_url":"data:image/png;base64,YWJj"}}),
            ),
        };
        let content = tool_result_content(&result);
        assert_eq!(content[0]["text"], "refused");
        assert_eq!(
            content[1]["source"],
            json!({"type":"base64","media_type":"image/png","data":"YWJj"})
        );
    }
}
