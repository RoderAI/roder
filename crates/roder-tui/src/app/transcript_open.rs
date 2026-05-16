use roder_app_server::LocalAppClient;
use roder_protocol::{JsonRpcRequest, TranscriptOpenFileParams, TranscriptOpenFileResult};

use super::decode_response;
use super::selection_keyboard::SelectionKeyboardState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TranscriptOpenOutcome {
    pub message: String,
    pub event: String,
}

pub(super) fn copy_url_fallback(
    clipboard: &mut SelectionKeyboardState,
    url: &str,
) -> anyhow::Result<TranscriptOpenOutcome> {
    clipboard.copy_text(url)?;
    Ok(TranscriptOpenOutcome {
        message: format!("system: copied URL {url}."),
        event: format!("url copied: {url}"),
    })
}

pub(super) async fn request_file_open(
    client: &LocalAppClient,
    thread_id: &str,
    path: &str,
    line: Option<u32>,
) -> anyhow::Result<TranscriptOpenOutcome> {
    let target = file_target(path, line);
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("transcript/open_file")),
            method: "transcript/open_file".to_string(),
            params: Some(serde_json::to_value(TranscriptOpenFileParams {
                thread_id: thread_id.to_string(),
                path: path.to_string(),
                line,
            })?),
        })
        .await;
    let result = decode_response::<TranscriptOpenFileResult>(res)?;
    if !result.requested {
        anyhow::bail!("transcript/open_file was not accepted");
    }
    Ok(TranscriptOpenOutcome {
        message: format!("system: requested file open {target}."),
        event: format!("file open requested: {target}"),
    })
}

pub(super) fn copy_file_fallback(
    clipboard: &mut SelectionKeyboardState,
    path: &str,
    line: Option<u32>,
) -> anyhow::Result<TranscriptOpenOutcome> {
    let target = file_target(path, line);
    clipboard.copy_text(&target)?;
    Ok(TranscriptOpenOutcome {
        message: format!("system: copied file reference {target}."),
        event: format!("file reference copied: {target}"),
    })
}

fn file_target(path: &str, line: Option<u32>) -> String {
    match line {
        Some(line) => format!("{path}:{line}"),
        None => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_fallback_copies_and_describes_target() {
        let mut clipboard = SelectionKeyboardState::default();

        let outcome = copy_url_fallback(&mut clipboard, "https://example.com").unwrap();

        assert_eq!(clipboard.paste_text(), Some("https://example.com"));
        assert_eq!(outcome.message, "system: copied URL https://example.com.");
        assert_eq!(outcome.event, "url copied: https://example.com");
    }

    #[test]
    fn file_fallback_preserves_line_number() {
        let mut clipboard = SelectionKeyboardState::default();

        let outcome =
            copy_file_fallback(&mut clipboard, "crates/roder-tui/src/app.rs", Some(42)).unwrap();

        assert_eq!(
            clipboard.paste_text(),
            Some("crates/roder-tui/src/app.rs:42")
        );
        assert_eq!(
            outcome.event,
            "file reference copied: crates/roder-tui/src/app.rs:42"
        );
    }
}
