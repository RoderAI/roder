//! Path-based `view_image` tool.
//!
//! Mirrors Codex's native `view_image(path)`: it reads an image file from the
//! workspace, base64-encodes it, and returns it as an image content block in
//! the tool result so the model receives the pixels as `input_image`. This is
//! the working replacement for `media_attach` (which demanded raw base64 the
//! model cannot supply) and the `chrome_*` tools (which need a live browser).

use std::sync::Arc;

use base64::Engine;
use roder_api::media::data_url;
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use roder_api::transcript::VIEW_IMAGE_DISPLAY_KEY;
use serde::Deserialize;
use serde_json::json;

use crate::backend::{WorkspaceBackendHandle, backend_from_context_or_fallback};
use crate::files::{parse, require_nonempty, result};
use crate::workspace::Workspace;

/// Raw byte ceiling for a viewable image. The Responses API caps images near
/// 20 MB and base64 inflates by ~4/3, so 10 MiB of source bytes stays safely
/// under the wire limit while covering any realistic screenshot or diagram.
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

pub(crate) fn register(
    registry: &mut ToolRegistry,
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
) -> anyhow::Result<()> {
    registry.register(Arc::new(ViewImageTool { workspace, backend }))
}

struct ViewImageTool {
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
}

#[derive(Debug, Deserialize)]
struct ViewImageArgs {
    path: String,
}

#[async_trait::async_trait]
impl ToolExecutor for ViewImageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "view_image".to_string(),
            description: "View an image file (png, jpeg, gif, or webp) so you can see its contents. \
                Provide the path to a local image and it is attached to the conversation as an image \
                you can inspect. Relative paths resolve from the workspace root."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the image file to view."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let args = parse::<ViewImageArgs>(&call)?;
        require_nonempty(&args.path, "path")?;
        let backend = backend_from_context_or_fallback(&ctx, &self.workspace, &self.backend)?;
        let (path, bytes) = backend.read_bytes(&args.path).await?;

        if bytes.len() > MAX_IMAGE_BYTES {
            return Ok(result(
                call,
                format!(
                    "image {path} is {} bytes, over the {} byte view_image limit",
                    bytes.len(),
                    MAX_IMAGE_BYTES
                ),
                json!({ "path": path }),
                true,
            ));
        }

        let Some(mime_type) = detect_image_mime(&bytes) else {
            return Ok(result(
                call,
                format!("{path} is not a supported image (expected png, jpeg, gif, or webp bytes)"),
                json!({ "path": path }),
                true,
            ));
        };

        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let url = data_url(mime_type, &encoded);
        let text = format!(
            "Viewing image {path} ({mime_type}, {} bytes). The image is attached below.",
            bytes.len()
        );
        Ok(result(
            call,
            text,
            json!({
                "path": path,
                "mime_type": mime_type,
                "byte_size": bytes.len(),
                VIEW_IMAGE_DISPLAY_KEY: {
                    "image_url": url,
                    "detail": "auto",
                },
            }),
            false,
        ))
    }
}

/// Identify the image format from its magic bytes, returning the MIME type.
/// Detection is content-based rather than extension-based so a mislabeled or
/// extensionless file is still handled correctly (and non-images are rejected).
fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.starts_with(PNG) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::policy_mode::PolicyMode;
    use roder_api::tools::LocalWorkspaceHandle;
    use roder_api::transcript::tool_display_payload;

    fn context(root: &std::path::Path) -> ToolExecutionContext {
        ToolExecutionContext::new("thread", "turn", PolicyMode::Default)
            .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(root)))
    }

    fn temp_workspace(prefix: &str) -> (std::path::PathBuf, Workspace, WorkspaceBackendHandle) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::new(root.clone()).unwrap();
        let backend: WorkspaceBackendHandle = Arc::new(crate::backend::LocalWorkspaceBackend::new(
            workspace.clone(),
        ));
        (root, workspace, backend)
    }

    fn call(path: &str) -> ToolCall {
        ToolCall {
            id: "call".to_string(),
            name: "view_image".to_string(),
            arguments: json!({ "path": path }),
            raw_arguments: String::new(),
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
        }
    }

    // 1x1 transparent PNG.
    const PNG_BYTES: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[tokio::test]
    async fn returns_image_content_block_for_png() {
        let (root, workspace, backend) = temp_workspace("view-image-png");
        std::fs::write(root.join("board.png"), PNG_BYTES).unwrap();

        let tool = ViewImageTool { workspace, backend };
        let result = tool
            .execute(context(&root), call("board.png"))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["mime_type"], "image/png");
        let image_url = result.data[VIEW_IMAGE_DISPLAY_KEY]["image_url"]
            .as_str()
            .unwrap();
        assert!(image_url.starts_with("data:image/png;base64,"));

        // The image block must survive the display-payload projection so the
        // provider can forward it to the model.
        let display = tool_display_payload(Some("view_image"), None, Some(&result.data)).unwrap();
        assert!(display[VIEW_IMAGE_DISPLAY_KEY]["image_url"].is_string());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn rejects_non_image_files() {
        let (root, workspace, backend) = temp_workspace("view-image-nonimage");
        std::fs::write(root.join("notes.txt"), b"just text, not an image").unwrap();

        let tool = ViewImageTool { workspace, backend };
        let result = tool
            .execute(context(&root), call("notes.txt"))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.data.get(VIEW_IMAGE_DISPLAY_KEY).is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn detects_supported_formats() {
        assert_eq!(detect_image_mime(PNG_BYTES), Some("image/png"));
        assert_eq!(
            detect_image_mime(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("image/jpeg")
        );
        assert_eq!(detect_image_mime(b"GIF89a....."), Some("image/gif"));
        assert_eq!(
            detect_image_mime(b"RIFF\0\0\0\0WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(detect_image_mime(b"plain text"), None);
    }
}
