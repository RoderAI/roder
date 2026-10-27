use std::path::PathBuf;

use serde_json::Value;

const MAX_LOCAL_ARTIFACTS: usize = 4;

/// Filesystem identity of tool arguments, independent of the TUI process directory.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::app) enum ImageOrigin {
    #[default]
    Unknown,
    Local {
        cwd: PathBuf,
    },
    Remote,
}

impl ImageOrigin {
    pub fn from_thread(thread: &roder_protocol::Thread) -> Self {
        if thread.runner.is_some() {
            Self::Remote
        } else {
            Self::local(PathBuf::from(&thread.cwd))
        }
    }

    pub fn local(cwd: PathBuf) -> Self {
        if cwd.is_absolute() {
            Self::Local { cwd }
        } else {
            Self::Unknown
        }
    }

    fn resolve_view_path(&self, path: &str) -> Option<PathBuf> {
        let Self::Local { cwd } = self else {
            return None;
        };
        if !cwd.is_absolute() || path.is_empty() || path.contains("://") {
            return None;
        }
        let path = PathBuf::from(path);
        Some(if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        })
    }
}

/// Only explicit image tools supply local paths. Never interpret arbitrary assistant prose,
/// URLs, or shell output as a filesystem read request.
pub(in crate::app) fn tool_image_paths(
    origin: &ImageOrigin,
    name: &str,
    arguments: &str,
    output: Option<&str>,
) -> Vec<PathBuf> {
    let name = name.rsplit(['/', '.']).next().unwrap_or(name);
    match name {
        "view_image" => serde_json::from_str::<Value>(arguments)
            .ok()
            .and_then(|value| origin.resolve_view_path(value.get("path")?.as_str()?))
            .into_iter()
            .collect(),
        "media_generate_image" | "media_attach" => {
            let Some(value) = output.and_then(|output| serde_json::from_str::<Value>(output).ok())
            else {
                return Vec::new();
            };
            let mut paths = Vec::new();
            collect_artifact_paths(&value, &mut paths, 0);
            paths
        }
        _ => Vec::new(),
    }
}

fn collect_artifact_paths(value: &Value, paths: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 || paths.len() >= MAX_LOCAL_ARTIFACTS {
        return;
    }
    match value {
        Value::Object(object) => {
            for key in ["thumbnail_path", "store_path"] {
                if let Some(path) = object.get(key).and_then(Value::as_str) {
                    // These explicit artifact records refer to host-owned media storage.
                    // Never interpret a relative or remote-store key using the UI cwd.
                    if PathBuf::from(path).is_absolute() && !path.contains("://") {
                        paths.push(PathBuf::from(path));
                        break;
                    }
                }
            }
            for key in [
                "artifact",
                "artifacts",
                "media",
                "result",
                "data",
                "mediaArtifacts",
                "mediaGeneration",
                "outputs",
            ] {
                if let Some(value) = object.get(key) {
                    collect_artifact_paths(value, paths, depth + 1);
                }
            }
        }
        Value::Array(values) => {
            for value in values.iter().take(MAX_LOCAL_ARTIFACTS) {
                collect_artifact_paths(value, paths, depth + 1);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_view_paths_use_thread_workspace_not_ui_process_directory() {
        let process_cwd = std::env::current_dir().unwrap();
        let thread_cwd = process_cwd.join("different-thread-workspace");
        let origin = ImageOrigin::local(thread_cwd.clone());
        let paths = tool_image_paths(
            &origin,
            "functions.view_image",
            r#"{"path":"screenshots/a.png"}"#,
            None,
        );
        assert_eq!(paths, vec![thread_cwd.join("screenshots/a.png")]);
        assert_ne!(paths[0], process_cwd.join("screenshots/a.png"));
        assert_eq!(
            tool_image_paths(&origin, "view_image", r#"{"path":"/explicit/a.png"}"#, None),
            vec![PathBuf::from("/explicit/a.png")],
        );
    }

    #[test]
    fn remote_and_unknown_view_arguments_never_select_a_host_file() {
        for origin in [ImageOrigin::Remote, ImageOrigin::Unknown] {
            for arguments in [
                r#"{"path":"/tmp/exists-on-both-hosts.png"}"#,
                r#"{"path":"a.png"}"#,
            ] {
                assert!(tool_image_paths(&origin, "view_image", arguments, None).is_empty());
            }
        }
        assert_eq!(ImageOrigin::local(PathBuf::from(".")), ImageOrigin::Unknown);
        assert!(
            tool_image_paths(
                &ImageOrigin::Local { cwd: ".".into() },
                "view_image",
                r#"{"path":"a.png"}"#,
                None,
            )
            .is_empty()
        );
    }

    #[test]
    fn only_explicit_image_tools_and_local_artifact_paths_are_discovered() {
        let origin = ImageOrigin::local(PathBuf::from("/thread"));
        assert!(tool_image_paths(&origin, "shell", r#"{"path":"/tmp/secret"}"#, None).is_empty());
        assert!(
            tool_image_paths(
                &origin,
                "view_image",
                r#"{"path":"https://example.com/a.png"}"#,
                None,
            )
            .is_empty()
        );
        for origin in [ImageOrigin::Unknown, ImageOrigin::Remote] {
            assert_eq!(
                tool_image_paths(
                    &origin,
                    "media_generate_image",
                    "{}",
                    Some(
                        r#"{"artifacts":[{"store_path":"/tmp/a.png"},{"store_path":"memory://a"},{"store_path":"relative.png"}]}"#
                    )
                ),
                vec![PathBuf::from("/tmp/a.png")],
            );
        }
    }

    #[test]
    fn thread_origin_uses_authoritative_runner_binding_and_absolute_cwd() {
        let mut thread: roder_protocol::Thread = serde_json::from_value(serde_json::json!({
            "id": "thread-1",
            "preview": "",
            "modelProvider": "test",
            "model": "test",
            "createdAt": 0,
            "updatedAt": 0,
            "status": {"type": "idle", "activeFlags": []},
            "cwd": "/thread/workspace",
        }))
        .unwrap();
        assert_eq!(
            ImageOrigin::from_thread(&thread),
            ImageOrigin::Local {
                cwd: PathBuf::from("/thread/workspace")
            },
        );
        thread.runner = Some(roder_protocol::ThreadRunnerParams {
            provider_id: "remote-provider".into(),
            config: None,
            workspace: "/thread/workspace".into(),
            read_roots: Vec::new(),
        });
        assert_eq!(ImageOrigin::from_thread(&thread), ImageOrigin::Remote);
        thread.runner = None;
        thread.cwd = "relative-workspace".into();
        assert_eq!(ImageOrigin::from_thread(&thread), ImageOrigin::Unknown);
    }
}
