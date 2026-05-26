use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::proto::{
    encode_connect_frame, is_context_frame, proto_field_bytes, proto_field_string, proto_message,
};

const MAX_CONTEXT_FILE_BYTES: u64 = 128 * 1024;
const TARGET_CONTEXT_TEXT_BYTES: usize = 120 * 1024;
const MAX_CONTEXT_TEXT_BYTES: usize = 240 * 1024;

#[derive(Debug, Clone)]
pub struct CursorContextOptions {
    pub context_files: Vec<CursorContextFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorContextFile {
    pub path: String,
    pub text: String,
}

impl CursorContextOptions {
    pub fn from_workspace(workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        let context_files = discover_context_files(&workspace_root);
        Self { context_files }
    }
}

pub fn encode_request_context_frame(options: &CursorContextOptions) -> Vec<u8> {
    let mut fields = Vec::new();
    if options.context_files.is_empty() {
        fields.push(proto_field_bytes(
            2,
            encode_context_item(
                "roder-cursor-context.txt",
                "Roder Cursor direct AgentService inference context.",
            ),
        ));
    } else {
        for file in &options.context_files {
            fields.push(proto_field_bytes(
                2,
                encode_context_item(&file.path, &file.text),
            ));
        }
    }
    let request_context = proto_message(fields);
    let envelope = proto_message(vec![proto_field_bytes(
        2,
        proto_message(vec![proto_field_bytes(
            10,
            proto_message(vec![proto_field_bytes(
                1,
                proto_message(vec![proto_field_bytes(1, request_context)]),
            )]),
        )]),
    )]);
    encode_connect_frame(&envelope)
}

pub fn discovery_context_frames_from_env() -> anyhow::Result<Option<Vec<Vec<u8>>>> {
    if std::env::var("RODER_CURSOR_ALLOW_DISCOVERY_CONTEXT")
        .ok()
        .as_deref()
        != Some("1")
    {
        return Ok(None);
    }
    let Some(path) = std::env::var("RODER_CURSOR_CONTEXT_TRACE_JSONL")
        .ok()
        .or_else(|| std::env::var("CURSOR_CONTEXT_TRACE_JSONL").ok())
    else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(path)?;
    let mut frames = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value = serde_json::from_str(line)?;
        let is_agent_run = event.get("type").and_then(Value::as_str) == Some("http2.request.write")
            && event
                .pointer("/headers/:path")
                .and_then(Value::as_str)
                .is_some_and(|path| path == "/agent.v1.AgentService/Run");
        if !is_agent_run {
            continue;
        }
        let Some(hex) = event.get("chunkHex").and_then(Value::as_str) else {
            continue;
        };
        let frame = hex_to_bytes(hex)?;
        if is_context_frame(&frame) {
            frames.push(frame);
        }
    }
    Ok((!frames.is_empty()).then_some(frames))
}

fn encode_context_item(path: &str, text: &str) -> Vec<u8> {
    proto_message(vec![
        proto_field_string(1, path),
        proto_field_string(2, text),
        proto_field_bytes(
            3,
            proto_message(vec![proto_field_bytes(
                3,
                proto_message(vec![proto_field_string(1, text)]),
            )]),
        ),
    ])
}

fn discover_context_files(root: &Path) -> Vec<CursorContextFile> {
    let mut files = Vec::new();
    for candidate in [
        "AGENTS.md",
        ".agents/AGENTS.md",
        ".cursor/rules",
        ".agents/skills",
        "roadmap/68-roder-cursor-api-provider.md",
        "roadmap/00-feature-inventory-and-sequencing.md",
        "roadmap/STATUS.md",
        "README.md",
        "Cargo.toml",
    ] {
        push_context_files(root, candidate, &mut files);
        if context_text_bytes(&files) >= TARGET_CONTEXT_TEXT_BYTES {
            break;
        }
    }
    files
}

fn push_context_files(root: &Path, candidate: &str, files: &mut Vec<CursorContextFile>) {
    let path = root.join(candidate);
    if path.is_dir() {
        if candidate.ends_with("skills") {
            push_skill_files(root, &path, files);
        } else {
            push_directory_files(root, &path, files);
        }
        return;
    }
    push_context_file(root, path, files);
}

fn push_skill_files(root: &Path, path: &Path, files: &mut Vec<CursorContextFile>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    let mut entries = entries.flatten().collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let skill_path = entry.path().join("SKILL.md");
        push_context_file(root, skill_path, files);
        if context_text_bytes(files) >= TARGET_CONTEXT_TEXT_BYTES {
            break;
        }
    }
}

fn push_directory_files(root: &Path, path: &Path, files: &mut Vec<CursorContextFile>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    let mut entries = entries.flatten().collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        push_context_file(root, entry.path(), files);
        if context_text_bytes(files) >= TARGET_CONTEXT_TEXT_BYTES {
            break;
        }
    }
}

fn push_context_file(root: &Path, path: PathBuf, files: &mut Vec<CursorContextFile>) {
    if context_text_bytes(files) >= MAX_CONTEXT_TEXT_BYTES {
        return;
    }
    let Some(file) = read_context_file(root, path) else {
        return;
    };
    let next_bytes = context_text_bytes(files).saturating_add(file.text.len());
    if !files.is_empty() && next_bytes > MAX_CONTEXT_TEXT_BYTES {
        return;
    }
    files.push(file);
}

fn context_text_bytes(files: &[CursorContextFile]) -> usize {
    files.iter().map(|file| file.text.len()).sum()
}

fn read_context_file(root: &Path, path: PathBuf) -> Option<CursorContextFile> {
    let metadata = std::fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_CONTEXT_FILE_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    let relative = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .to_string();
    Some(CursorContextFile {
        path: relative,
        text,
    })
}

fn hex_to_bytes(hex: &str) -> anyhow::Result<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        anyhow::bail!("hex string has odd length");
    }
    hex.as_bytes()
        .chunks(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair)?;
            Ok(u8::from_str_radix(text, 16)?)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_context_frame_is_connect_framed() {
        let frame = encode_request_context_frame(&CursorContextOptions {
            context_files: vec![CursorContextFile {
                path: "AGENTS.md".to_string(),
                text: "repo rules".to_string(),
            }],
        });
        assert_eq!(frame[0], 0);
        assert!(frame.windows("AGENTS.md".len()).any(|w| w == b"AGENTS.md"));
        assert!(
            frame
                .windows("repo rules".len())
                .any(|w| w == b"repo rules")
        );
    }

    #[test]
    fn discovery_context_is_disabled_by_default() {
        assert!(discovery_context_frames_from_env().unwrap().is_none());
    }
}
