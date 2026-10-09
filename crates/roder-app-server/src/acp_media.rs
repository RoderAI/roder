use agent_client_protocol_schema as acp;
use serde_json::Value;

pub(crate) fn tool_image(payload: Option<&Value>) -> Option<acp::ToolCallContent> {
    let (mime, data) = roder_api::transcript::tool_result_image(payload)?;
    Some(acp::ToolCallContent::from(acp::ContentBlock::Image(
        acp::ImageContent::new(data, mime),
    )))
}

pub(crate) fn desktop_kind(name: &str) -> Option<acp::ToolKind> {
    let name = name.strip_prefix("cua_")?;
    Some(
        if matches!(
            name,
            "get_window_state"
                | "get_desktop_state"
                | "get_browser_state"
                | "list_windows"
                | "list_apps"
        ) {
            acp::ToolKind::Read
        } else {
            acp::ToolKind::Execute
        },
    )
}
