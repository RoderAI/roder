use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use roder_api::code_index::{CodeByteRange, CodeChunk, CodeLineRange};

use crate::hex_sha256;
use crate::merkle::{FileManifestEntry, hash_path};

const MAX_CHUNK_LINES: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkedFile {
    pub path: PathBuf,
    pub chunks: Vec<CodeChunk>,
}

pub fn chunk_workspace(
    workspace_root: impl AsRef<Path>,
    files: &[FileManifestEntry],
) -> anyhow::Result<Vec<CodeChunk>> {
    let workspace_root = workspace_root.as_ref();
    let mut chunks = Vec::new();
    for file in files {
        let path = workspace_root.join(&file.path);
        let bytes = fs::read(&path)
            .with_context(|| format!("read source chunk file {}", path.display()))?;
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        chunks.extend(chunk_text_file(&file.path, &text));
    }
    Ok(chunks)
}

pub fn chunk_text_file(path: impl AsRef<Path>, text: &str) -> Vec<CodeChunk> {
    let path = path.as_ref();
    if text.is_empty() {
        return Vec::new();
    }

    let lines = line_spans(text);
    if lines.is_empty() {
        return Vec::new();
    }

    let mut boundaries = vec![0usize];
    for (idx, line) in lines.iter().enumerate().skip(1) {
        if is_symbol_boundary(&text[line.start..line.end]) {
            boundaries.push(idx);
        }
    }
    push_fallback_boundaries(&mut boundaries, lines.len());
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut chunks = Vec::new();
    for (boundary_idx, start_line_idx) in boundaries.iter().enumerate() {
        let end_line_idx = boundaries
            .get(boundary_idx + 1)
            .copied()
            .unwrap_or(lines.len());
        if end_line_idx <= *start_line_idx {
            continue;
        }
        let start_byte = lines[*start_line_idx].start;
        let end_byte = lines[end_line_idx - 1].end_with_newline;
        if start_byte >= end_byte {
            continue;
        }
        let source = &text[start_byte..end_byte];
        let content_hash = hex_sha256(source);
        let language = language_for_path(path);
        let symbol_hint = symbol_hint(source);
        let path_hash = hash_path(path);
        let chunk_material = format!(
            "{}\0{}\0{}\0{}",
            path.display(),
            start_byte,
            end_byte,
            content_hash
        );
        chunks.push(CodeChunk {
            chunk_hash: hex_sha256(chunk_material),
            path: path.to_path_buf(),
            path_hash,
            byte_range: CodeByteRange {
                start: start_byte as u64,
                end: end_byte as u64,
            },
            line_range: CodeLineRange {
                start: (*start_line_idx + 1) as u32,
                end: end_line_idx as u32,
            },
            content_hash,
            language: language.map(str::to_string),
            symbol_hint,
        });
    }

    chunks
}

#[derive(Debug, Clone, Copy)]
struct LineSpan {
    start: usize,
    end: usize,
    end_with_newline: usize,
}

fn line_spans(text: &str) -> Vec<LineSpan> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    for line in text.split_inclusive('\n') {
        let end_with_newline = start + line.len();
        let end = line.strip_suffix('\n').map_or(end_with_newline, |trimmed| {
            start + trimmed.trim_end_matches('\r').len()
        });
        spans.push(LineSpan {
            start,
            end,
            end_with_newline,
        });
        start = end_with_newline;
    }
    if start < text.len() {
        spans.push(LineSpan {
            start,
            end: text.len(),
            end_with_newline: text.len(),
        });
    }
    spans
}

fn push_fallback_boundaries(boundaries: &mut Vec<usize>, line_count: usize) {
    let mut cursor = MAX_CHUNK_LINES;
    while cursor < line_count {
        boundaries.push(cursor);
        cursor += MAX_CHUNK_LINES;
    }
}

fn is_symbol_boundary(line: &str) -> bool {
    let trimmed = line.trim_start();
    SYMBOL_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

fn symbol_hint(source: &str) -> Option<String> {
    source.lines().find_map(|line| {
        let trimmed = line.trim_start();
        let matched = SYMBOL_PREFIXES
            .iter()
            .find(|prefix| trimmed.starts_with(**prefix))?;
        let rest = trimmed.trim_start_matches(matched).trim_start();
        let name = rest
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .find(|part| !part.is_empty())?;
        Some(name.to_string())
    })
}

fn language_for_path(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") => Some("rust"),
        Some("go") => Some("go"),
        Some("ts") => Some("typescript"),
        Some("tsx") => Some("tsx"),
        Some("js") => Some("javascript"),
        Some("jsx") => Some("jsx"),
        Some("py") => Some("python"),
        Some("java") => Some("java"),
        Some("kt") => Some("kotlin"),
        Some("swift") => Some("swift"),
        _ => None,
    }
}

const SYMBOL_PREFIXES: &[&str] = &[
    "pub async fn ",
    "pub fn ",
    "async fn ",
    "fn ",
    "pub struct ",
    "struct ",
    "pub enum ",
    "enum ",
    "pub trait ",
    "trait ",
    "impl ",
    "class ",
    "def ",
    "function ",
    "export function ",
    "const ",
    "let ",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_uses_symbol_boundaries_and_line_ranges() {
        let source = "pub fn first() {\n}\n\npub struct Second;\nimpl Second {\n}\n";
        let chunks = chunk_text_file("src/lib.rs", source);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].line_range.start, 1);
        assert_eq!(chunks[0].line_range.end, 3);
        assert_eq!(chunks[1].line_range.start, 4);
        assert_eq!(chunks[1].symbol_hint.as_deref(), Some("Second"));
        assert_eq!(chunks[2].line_range.start, 5);
        assert_eq!(chunks[2].symbol_hint.as_deref(), Some("Second"));
    }

    #[test]
    fn chunk_falls_back_to_bounded_line_ranges() {
        let source = (0..170)
            .map(|idx| format!("// line {idx}\n"))
            .collect::<String>();
        let chunks = chunk_text_file("notes.txt", &source);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].line_range, CodeLineRange { start: 1, end: 80 });
        assert_eq!(
            chunks[1].line_range,
            CodeLineRange {
                start: 81,
                end: 160
            }
        );
        assert_eq!(
            chunks[2].line_range,
            CodeLineRange {
                start: 161,
                end: 170
            }
        );
    }
}
