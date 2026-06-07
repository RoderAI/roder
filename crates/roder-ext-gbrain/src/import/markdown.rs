use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{Value, json};

use super::{JsonlImportRecord, LoadedImportBatch, LoadedImportRecord, ensure_directory_format};

const MAX_MARKDOWN_FILE_BYTES: u64 = 5_000_000;
const PRUNE_DIR_NAMES: &[&str] = &["node_modules", ".raw", "ops"];
const SKIP_MARKDOWN_BASENAMES: &[&str] = &["schema.md", "index.md", "log.md", "README.md"];

pub(super) fn load_markdown_corpus_dir(
    path: &Path,
    format: &str,
) -> anyhow::Result<LoadedImportBatch> {
    ensure_directory_format(format)?;
    let mut files = Vec::new();
    collect_markdown_files(path, &mut files)?;
    files.sort();

    let mut records = Vec::new();
    let mut hash_body = String::new();
    let mut errors = 0usize;
    for file in files {
        let metadata =
            std::fs::symlink_metadata(&file).with_context(|| format!("stat {}", file.display()))?;
        if metadata.file_type().is_symlink() || metadata.len() > MAX_MARKDOWN_FILE_BYTES {
            errors += 1;
            continue;
        }
        let text =
            std::fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
        hash_body.push_str(
            &file
                .strip_prefix(path)
                .unwrap_or(&file)
                .display()
                .to_string(),
        );
        hash_body.push('\n');
        hash_body.push_str(&crate::model::content_hash(&text));
        hash_body.push('\n');
        match parse_markdown_artifact(path, &file, &text) {
            Ok(record) => records.push(LoadedImportRecord {
                line_index: records.len(),
                record,
            }),
            Err(_) => errors += 1,
        }
    }

    Ok(LoadedImportBatch {
        source_path: Some(path.display().to_string()),
        source_hash: crate::model::content_hash(&hash_body),
        records,
        errors,
    })
}

fn collect_markdown_files(path: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(path).with_context(|| format!("read dir {}", path.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if should_prune_dir(&name) {
                continue;
            }
            collect_markdown_files(&path, out)?;
        } else if file_type.is_file()
            && is_markdown_file(&path)
            && !should_skip_markdown_file(&name)
        {
            out.push(path);
        }
    }
    Ok(())
}

fn should_prune_dir(name: &str) -> bool {
    name.starts_with('.') || PRUNE_DIR_NAMES.contains(&name) || name.ends_with(".raw")
}

fn should_skip_markdown_file(name: &str) -> bool {
    SKIP_MARKDOWN_BASENAMES.contains(&name)
}

fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| matches!(extension.to_ascii_lowercase().as_str(), "md" | "mdx"))
        .unwrap_or(false)
}

fn parse_markdown_artifact(
    root: &Path,
    path: &Path,
    raw: &str,
) -> anyhow::Result<JsonlImportRecord> {
    let rel_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let (mut metadata, body) = split_markdown_header(raw);
    let file_slug = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let header_family = metadata.get("header_family").cloned();
    if is_path_inferred_markdown(header_family.as_deref()) {
        metadata
            .entry("type".to_string())
            .or_insert_with(|| infer_source_type(&rel_path).to_string());
        metadata
            .entry("title".to_string())
            .or_insert_with(|| infer_title(&rel_path));
    }
    let path_slug = slugify_path(&rel_path);
    let default_slug = if header_family.as_deref() == Some("markdown_frontmatter")
        || header_family.as_deref() == Some("markdown")
    {
        metadata
            .get("slug")
            .cloned()
            .unwrap_or_else(|| path_slug.clone())
    } else {
        file_slug.clone()
    };
    let slot_id = metadata
        .entry("slot_id".to_string())
        .or_insert_with(|| default_slug.clone())
        .clone();
    if !metadata.contains_key("event_id")
        && let Some(parent) = path
            .parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| name.starts_with("EV-"))
    {
        metadata.insert("event_id".into(), parent);
    }
    let event_id = metadata.get("event_id").cloned();
    let thread_id = event_id
        .clone()
        .or_else(|| metadata.get("pattern").cloned())
        .or_else(|| metadata.get("customer").cloned())
        .unwrap_or_else(|| slot_id.clone());
    let timestamp = metadata
        .get("header_date")
        .filter(|value| is_full_date(value))
        .cloned()
        .or_else(|| {
            metadata
                .get("date")
                .and_then(|value| first_full_date(value))
        })
        .or_else(|| {
            metadata
                .get("created")
                .and_then(|value| first_full_date(value))
        })
        .or_else(|| {
            metadata
                .get("updated")
                .and_then(|value| first_full_date(value))
        })
        .or_else(|| {
            metadata
                .get("effective_date")
                .and_then(|value| first_full_date(value))
        })
        .or_else(|| first_date_line(body));
    let source_type = metadata
        .get("genre")
        .or_else(|| metadata.get("type"))
        .or_else(|| metadata.get("doc_type"))
        .or_else(|| metadata.get("header_family"))
        .cloned();
    let mut record_metadata = serde_json::Map::new();
    record_metadata.insert("source_path".into(), json!(rel_path));
    for (key, value) in &metadata {
        record_metadata.insert(key.clone(), json!(value));
    }
    if let Some(source_type) = &source_type {
        record_metadata.insert("source_type".into(), json!(source_type));
    }
    if let Some(event_id) = &event_id {
        record_metadata.insert("event_id".into(), json!(event_id));
    }
    record_metadata.insert("thread_id".into(), json!(thread_id));

    Ok(JsonlImportRecord {
        text: body.trim().to_string(),
        slug: Some(slot_id.clone()),
        source_id: Some(rel_path.clone()),
        subject: metadata.get("title").cloned(),
        timestamp: timestamp.clone(),
        valid_at: timestamp.clone(),
        invalid_at: None,
        ingested_at: timestamp,
        thread_id: Some(thread_id),
        provenance: vec![slot_id, rel_path],
        metadata: Value::Object(record_metadata),
    })
}

fn split_markdown_header(raw: &str) -> (BTreeMap<String, String>, &str) {
    if let Some(rest) = raw.strip_prefix("<!--")
        && let Some((header, body)) = rest.split_once("-->")
    {
        return split_comment_header(header.trim(), body);
    }
    if let Some((metadata, body)) = split_yaml_frontmatter(raw) {
        return (metadata, body);
    }
    let mut metadata = BTreeMap::new();
    metadata.insert("header_family".into(), "markdown".into());
    (metadata, raw)
}

fn split_comment_header<'a>(header: &str, body: &'a str) -> (BTreeMap<String, String>, &'a str) {
    let mut metadata = BTreeMap::new();
    if header.starts_with("artefact_metadata") {
        metadata.insert("header_family".into(), "artefact_metadata".into());
        parse_multiline_header(header, &mut metadata);
    } else {
        parse_inline_header(header, &mut metadata);
    }
    (metadata, body)
}

fn split_yaml_frontmatter(raw: &str) -> Option<(BTreeMap<String, String>, &str)> {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let mut offset = 0usize;
    let mut lines = raw.split_inclusive('\n');
    let first = lines.next()?;
    offset += first.len();
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return None;
    }
    let mut header = String::new();
    for line in lines {
        let line_end = offset + line.len();
        if line.trim_end_matches(['\r', '\n']) == "---" {
            let mut metadata = BTreeMap::new();
            metadata.insert("header_family".into(), "markdown_frontmatter".into());
            parse_key_value_lines(&header, &mut metadata);
            return Some((metadata, &raw[line_end..]));
        }
        header.push_str(line);
        offset = line_end;
    }
    None
}

fn parse_multiline_header(header: &str, metadata: &mut BTreeMap<String, String>) {
    for line in header.lines().skip(1) {
        parse_key_value_line(line, metadata);
    }
}

fn parse_key_value_lines(header: &str, metadata: &mut BTreeMap<String, String>) {
    for line in header.lines() {
        parse_key_value_line(line, metadata);
    }
}

fn parse_key_value_line(line: &str, metadata: &mut BTreeMap<String, String>) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    if let Some((key, value)) = line.split_once(':') {
        let value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        metadata.insert(key.trim().to_string(), value);
    }
}

fn parse_inline_header(header: &str, metadata: &mut BTreeMap<String, String>) {
    let mut segments = header.split_whitespace();
    let Some(family) = segments.next() else {
        return;
    };
    metadata.insert("header_family".into(), family.to_string());
    let mut current_key: Option<String> = None;
    let mut current_value = String::new();
    let mut header_args = Vec::new();
    for segment in segments {
        if let Some((key, value)) = segment.split_once('=') {
            flush_header_pair(metadata, &mut current_key, &mut current_value);
            current_key = Some(key.trim().to_string());
            current_value = value.trim().to_string();
        } else if is_period_like(segment) {
            flush_header_pair(metadata, &mut current_key, &mut current_value);
            if is_full_date(segment) {
                metadata.insert("header_date".into(), segment.to_string());
            } else {
                metadata.insert("header_period".into(), segment.to_string());
            }
        } else if current_key.is_some() {
            if !current_value.is_empty() {
                current_value.push(' ');
            }
            current_value.push_str(segment);
        } else {
            header_args.push(segment.to_string());
        }
    }
    flush_header_pair(metadata, &mut current_key, &mut current_value);
    if !header_args.is_empty() {
        metadata.insert("header_args".into(), header_args.join(" "));
    }
}

fn flush_header_pair(
    metadata: &mut BTreeMap<String, String>,
    current_key: &mut Option<String>,
    current_value: &mut String,
) {
    if let Some(key) = current_key.take() {
        metadata.insert(key, std::mem::take(current_value));
    }
}

fn first_date_line(body: &str) -> Option<String> {
    body.lines().map(str::trim).find_map(first_full_date)
}

fn first_full_date(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    for window in bytes.windows(10) {
        if window[4] == b'-'
            && window[7] == b'-'
            && window
                .iter()
                .enumerate()
                .all(|(idx, byte)| idx == 4 || idx == 7 || byte.is_ascii_digit())
        {
            return Some(String::from_utf8_lossy(window).to_string());
        }
    }
    None
}

fn is_full_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(idx, byte)| idx == 4 || idx == 7 || byte.is_ascii_digit())
}

fn is_period_like(value: &str) -> bool {
    is_full_date(value)
        || (value.len() == 7
            && value.as_bytes()[4] == b'-'
            && value.as_bytes()[5].is_ascii_digit()
            && value.as_bytes()[6].is_ascii_digit()
            && value[..4].bytes().all(|byte| byte.is_ascii_digit()))
        || (value.len() == 7
            && value.as_bytes()[4] == b'-'
            && value.as_bytes()[5] == b'Q'
            && value.as_bytes()[6].is_ascii_digit()
            && value[..4].bytes().all(|byte| byte.is_ascii_digit()))
}

fn slugify_path(path: &str) -> String {
    let without_ext = path
        .strip_suffix(".md")
        .or_else(|| path.strip_suffix(".MD"))
        .or_else(|| path.strip_suffix(".mdx"))
        .or_else(|| path.strip_suffix(".MDX"))
        .unwrap_or(path);
    without_ext
        .split('/')
        .filter_map(|segment| {
            let mut out = String::new();
            let mut last_dash = false;
            for ch in segment.chars().flat_map(char::to_lowercase) {
                if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                    out.push(ch);
                    last_dash = false;
                } else if ch.is_whitespace() && !last_dash {
                    out.push('-');
                    last_dash = true;
                }
            }
            let out = out.trim_matches('-').to_string();
            (!out.is_empty()).then_some(out)
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn is_path_inferred_markdown(header_family: Option<&str>) -> bool {
    matches!(header_family, Some("markdown_frontmatter" | "markdown"))
}

fn infer_source_type(path: &str) -> &'static str {
    let lower = format!("/{path}").to_ascii_lowercase();
    for (prefix, source_type) in [
        ("/writing/", "writing"),
        ("/wiki/analysis/", "analysis"),
        ("/wiki/guides/", "guide"),
        ("/wiki/guide/", "guide"),
        ("/wiki/hardware/", "hardware"),
        ("/wiki/architecture/", "architecture"),
        ("/wiki/concepts/", "concept"),
        ("/wiki/concept/", "concept"),
        ("/people/", "person"),
        ("/person/", "person"),
        ("/companies/", "company"),
        ("/company/", "company"),
        ("/deals/", "deal"),
        ("/deal/", "deal"),
        ("/yc/", "yc"),
        ("/civic/", "civic"),
        ("/projects/", "project"),
        ("/project/", "project"),
        ("/sources/", "source"),
        ("/source/", "source"),
        ("/media/", "media"),
        ("/emails/", "email"),
        ("/email/", "email"),
        ("/slack/", "slack"),
        ("/cal/", "calendar-event"),
        ("/calendar/", "calendar-event"),
        ("/notes/", "note"),
        ("/note/", "note"),
        ("/meetings/", "meeting"),
        ("/meeting/", "meeting"),
    ] {
        if lower.contains(prefix) {
            return source_type;
        }
    }
    "concept"
}

fn infer_title(path: &str) -> String {
    let filename = path.rsplit('/').next().unwrap_or("Untitled");
    let filename = filename
        .strip_suffix(".md")
        .or_else(|| filename.strip_suffix(".MD"))
        .or_else(|| filename.strip_suffix(".mdx"))
        .or_else(|| filename.strip_suffix(".MDX"))
        .unwrap_or(filename);
    let title = filename
        .replace(['-', '_'], " ")
        .split_whitespace()
        .map(capitalize_ascii_word)
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        "Untitled".to_string()
    } else {
        title
    }
}

fn capitalize_ascii_word(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_ascii_uppercase().to_string() + chars.as_str()
}
