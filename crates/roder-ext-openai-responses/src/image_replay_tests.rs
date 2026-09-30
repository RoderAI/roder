// Screenshot output must retain observations, call identity, and resolution.
use super::*;

fn view_image_transcript() -> Vec<TranscriptItem> {
    vec![
        TranscriptItem::ToolCall(ToolCallRecord {
            id: "call_img".to_string(),
            name: "view_image".to_string(),
            arguments: "{\"path\":\"board.png\"}".to_string(),
        }),
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "call_img".to_string(),
            name: Some("view_image".to_string()),
            result: "Viewing image board.png".to_string(),
            display_payload: Some(json!({
                "__view_image": {
                    "image_url": "data:image/png;base64,YWJj",
                    "detail": "original"
                }
            })),
            is_error: false,
        }),
    ]
}

#[test]
fn forwards_view_image_output_as_input_image() {
    let mut request = request();
    request.transcript = view_image_transcript();

    let input = input_items(&request);
    let output = input
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .expect("tool output present");
    let content = output["output"].as_array().expect("image array output");
    assert_eq!(output["call_id"], "call_img");
    assert_eq!(content[0]["type"], "input_text");
    assert_eq!(content[0]["text"], "Viewing image board.png");
    assert_eq!(content[1]["type"], "input_image");
    assert_eq!(content[1]["image_url"], "data:image/png;base64,YWJj");
    assert_eq!(content[1]["detail"], "original");
}

#[test]
fn view_image_output_falls_back_to_string_without_image_support() {
    let mut request = request();
    request.transcript = view_image_transcript();

    let (_, tool_name_map) = responses_tools(&request, ResponsesProviderProfile::OpenAi);
    let input = response_input_items(
        &request,
        &tool_name_map,
        ResponsesProviderProfile::OpenAi,
        false,
    );
    let output = input
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .expect("tool output present");
    assert_eq!(output["output"], "Viewing image board.png");
}
