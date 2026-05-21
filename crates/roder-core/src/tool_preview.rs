use roder_api::events::FileChangePreviewReady;
use roder_api::tools::ToolCall;
use time::OffsetDateTime;

pub(crate) fn file_change_preview(
    call: &ToolCall,
    workspace: Option<&str>,
) -> Option<FileChangePreviewReady> {
    let path = call.arguments.get("path")?.as_str()?;
    let full_path = resolve_workspace_path(workspace, path).ok()?;
    let before = std::fs::read_to_string(&full_path).ok();
    let after = match call.name.as_str() {
        "write_file" => call.arguments.get("content")?.as_str()?.to_string(),
        "edit" => {
            let before = before.as_ref()?;
            apply_single_edit(
                before,
                call.arguments.get("old_string")?.as_str()?,
                call.arguments.get("new_string")?.as_str()?,
            )?
        }
        "multi_edit" => {
            let mut text = before.as_ref()?.to_string();
            for edit in call.arguments.get("edits")?.as_array()? {
                text = apply_single_edit(
                    &text,
                    edit.get("old_string")?.as_str()?,
                    edit.get("new_string")?.as_str()?,
                )?;
            }
            text
        }
        _ => return None,
    };

    Some(FileChangePreviewReady {
        thread_id: call.thread_id.clone(),
        turn_id: call.turn_id.clone(),
        tool_id: call.id.clone(),
        tool_name: call.name.clone(),
        path: path.to_string(),
        change_type: if before.is_some() { "modify" } else { "create" }.to_string(),
        before,
        after,
        supports_partial: false,
        timestamp: OffsetDateTime::now_utc(),
    })
}

fn apply_single_edit(before: &str, old_string: &str, new_string: &str) -> Option<String> {
    let index = before.find(old_string)?;
    let mut after = before.to_string();
    after.replace_range(index..index + old_string.len(), new_string);
    Some(after)
}

fn resolve_workspace_path(
    workspace: Option<&str>,
    path: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let root = workspace
        .map(std::path::PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let path = std::path::PathBuf::from(path);
    let candidate = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    let normalized = normalize_path(candidate)?;
    let normalized_root = normalize_path(root)?;
    if !normalized.starts_with(&normalized_root) {
        anyhow::bail!("path escapes workspace");
    }
    Ok(normalized)
}

fn normalize_path(path: std::path::PathBuf) -> anyhow::Result<std::path::PathBuf> {
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            std::path::Component::RootDir => out.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    anyhow::bail!("path escapes workspace");
                }
            }
            std::path::Component::Normal(part) => out.push(part),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use roder_api::tools::ToolCall;
    use serde_json::json;

    use super::*;

    #[test]
    fn file_change_preview_for_write_file_reads_before_and_after() {
        let root = temp_workspace("write-file-preview");
        let path = root.join("src").join("lib.rs");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "old\n").unwrap();
        let call = tool_call(
            "write_file",
            json!({
                "path": "src/lib.rs",
                "content": "new\n"
            }),
        );

        let preview = file_change_preview(&call, Some(root.to_str().unwrap())).unwrap();

        assert_eq!(preview.thread_id, "thread-a");
        assert_eq!(preview.turn_id, "turn-a");
        assert_eq!(preview.tool_id, "tool-a");
        assert_eq!(preview.tool_name, "write_file");
        assert_eq!(preview.path, "src/lib.rs");
        assert_eq!(preview.change_type, "modify");
        assert_eq!(preview.before.as_deref(), Some("old\n"));
        assert_eq!(preview.after, "new\n");
        assert!(!preview.supports_partial);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn file_change_preview_rejects_paths_outside_workspace() {
        let root = temp_workspace("escaped-preview");
        let call = tool_call(
            "write_file",
            json!({
                "path": "../outside.txt",
                "content": "new\n"
            }),
        );

        assert!(file_change_preview(&call, Some(root.to_str().unwrap())).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    fn tool_call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "tool-a".to_string(),
            name: name.to_string(),
            raw_arguments: arguments.to_string(),
            arguments,
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
        }
    }

    fn temp_workspace(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("roder-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}

#[cfg(test)]
mod runtime_tests {
    use std::sync::Arc;

    use roder_api::events::{EventEnvelope, RoderEvent};
    use roder_api::extension::{ExtensionRegistryBuilder, ToolProviderId};
    use roder_api::inference::ToolCallCompleted as InferenceToolCallCompleted;
    use roder_api::policy_mode::PolicyMode;
    use roder_api::tools::{
        ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
        ToolSpec,
    };
    use serde_json::json;

    use crate::fake_provider::FakeInferenceEngine;
    use crate::runtime::{Runtime, RuntimeConfig};

    #[tokio::test]
    async fn file_change_preview_emits_before_tool_start() {
        let root = temp_workspace("runtime-preview");
        let path = root.join("src").join("lib.rs");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "old\n").unwrap();
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        builder.tool_contributor(Arc::new(TestToolContributor));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(root.to_string_lossy().into_owned()),
                policy_mode: PolicyMode::AcceptAll,
                ..RuntimeConfig::default()
            },
        )
        .unwrap();
        let mut rx = runtime.subscribe_events();

        runtime
            .route_tool_call(
                &"thread-a".to_string(),
                &"turn-a".to_string(),
                InferenceToolCallCompleted {
                    id: "tool-a".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({
                        "path": "src/lib.rs",
                        "content": "new\n"
                    })
                    .to_string(),
                },
                None,
                None,
            )
            .await
            .unwrap();

        let preview_seq = next_event_seq(&mut rx, "file.change_preview_ready").await;
        let started_seq = next_event_seq(&mut rx, "tool.call_started").await;
        assert!(preview_seq < started_seq);
        let _ = std::fs::remove_dir_all(root);
    }

    async fn next_event_seq(
        rx: &mut tokio::sync::broadcast::Receiver<EventEnvelope>,
        kind: &str,
    ) -> u64 {
        loop {
            let envelope = rx.recv().await.unwrap();
            if envelope.kind == kind {
                if kind == "file.change_preview_ready" {
                    assert!(matches!(
                        envelope.event,
                        RoderEvent::FileChangePreviewReady(_)
                    ));
                }
                return envelope.seq;
            }
        }
    }

    fn temp_workspace(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("roder-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    struct TestToolContributor;

    impl ToolContributor for TestToolContributor {
        fn id(&self) -> ToolProviderId {
            "test-tools".to_string()
        }

        fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
            registry.register(Arc::new(TestWriteFileTool))
        }
    }

    struct TestWriteFileTool;

    #[async_trait::async_trait]
    impl ToolExecutor for TestWriteFileTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "write_file".to_string(),
                description: "test write file".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    }
                }),
            }
        }

        async fn execute(
            &self,
            _ctx: ToolExecutionContext,
            call: ToolCall,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult {
                id: call.id,
                name: call.name,
                text: "ok".to_string(),
                data: json!({}),
                is_error: false,
            })
        }
    }
}
