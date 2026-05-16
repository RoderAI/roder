use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

use super::super::Theme;
use super::ToolTimelineEntry;

const MAX_PATCH_PREVIEW_LINES: usize = 80;

pub(super) fn tool_diff_preview_lines(
    entry: &ToolTimelineEntry,
    theme: Theme,
) -> Vec<Line<'static>> {
    let diff = match entry.name.as_str() {
        "apply_patch" => apply_patch_diff(&entry.arguments),
        "edit" => edit_diff(&entry.arguments),
        "multi_edit" => multi_edit_diff(&entry.arguments),
        "write_file" => write_file_diff(&entry.arguments),
        _ => None,
    };
    let Some(diff) = diff else {
        return Vec::new();
    };
    render_diff_lines(&diff, theme)
}

fn apply_patch_diff(arguments: &str) -> Option<String> {
    let patch = patch_argument(arguments)?;
    let patch = patch.trim_end();
    if patch.is_empty() {
        None
    } else {
        Some(patch.to_string())
    }
}

fn edit_diff(arguments: &str) -> Option<String> {
    let path = json_string_field(arguments, "path")?;
    let old = json_string_field(arguments, "old_string")?;
    let new = json_string_field(arguments, "new_string")?;
    Some(format_edit_diff(&path, &[(&old, &new)]))
}

fn multi_edit_diff(arguments: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(arguments).ok()?;
    let path = value.get("path")?.as_str()?;
    let edits = value.get("edits")?.as_array()?;
    if edits.is_empty() {
        return None;
    }
    let mut out = format!("*** Edit File: {path}\n");
    for edit in edits {
        let old = edit.get("old_string")?.as_str()?;
        let new = edit.get("new_string")?.as_str()?;
        push_edit_hunk(&mut out, old, new);
    }
    Some(out.trim_end().to_string())
}

fn write_file_diff(arguments: &str) -> Option<String> {
    let path = json_string_field(arguments, "path")?;
    let content = json_string_field(arguments, "content")?;
    let mut out = format!("*** Write File: {path}\n@@\n");
    for line in content.lines() {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    if content.is_empty() {
        out.push_str("+\n");
    }
    Some(out.trim_end().to_string())
}

fn render_diff_lines(diff: &str, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for line in diff.lines().take(MAX_PATCH_PREVIEW_LINES) {
        let style = patch_line_style(line, theme);
        lines.push(Line::from(vec![
            Span::styled("  ".to_string(), theme.subtle()),
            Span::styled(line.to_string(), style),
        ]));
    }
    if diff.lines().count() > MAX_PATCH_PREVIEW_LINES {
        lines.push(Line::from(vec![
            Span::styled("  ".to_string(), theme.subtle()),
            Span::styled(
                "diff preview truncated in timeline",
                theme.muted().add_modifier(Modifier::ITALIC),
            ),
        ]));
    }
    lines
}

fn format_edit_diff(path: &str, edits: &[(&str, &str)]) -> String {
    let mut out = format!("*** Edit File: {path}\n");
    for (old, new) in edits {
        push_edit_hunk(&mut out, old, new);
    }
    out.trim_end().to_string()
}

fn push_edit_hunk(out: &mut String, old: &str, new: &str) {
    out.push_str("@@\n");
    for line in old.lines() {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in new.lines() {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    if old.is_empty() && new.is_empty() {
        out.push_str(" \n");
    }
}

fn patch_line_style(line: &str, theme: Theme) -> Style {
    if line.starts_with('+') && !line.starts_with("+++") {
        return theme.policy_mode(roder_api::policy_mode::PolicyMode::AcceptEdits);
    }
    if line.starts_with('-') && !line.starts_with("---") {
        return theme.error();
    }
    if line.starts_with("@@")
        || line.starts_with("diff --git")
        || line.starts_with("*** Add File: ")
        || line.starts_with("*** Delete File: ")
        || line.starts_with("*** Update File: ")
        || line.starts_with("*** Edit File: ")
        || line.starts_with("*** Write File: ")
        || line.starts_with("*** Move to: ")
    {
        return theme.tool();
    }
    if line.starts_with("*** ") || line.starts_with("+++") || line.starts_with("---") {
        return theme.subtle();
    }
    theme.muted()
}

fn json_string_field(arguments: &str, field: &str) -> Option<String> {
    serde_json::from_str::<Value>(arguments)
        .ok()
        .and_then(|value| value.get(field).and_then(Value::as_str).map(str::to_string))
        .or_else(|| partial_json_string_field(arguments, field))
}

fn patch_argument(arguments: &str) -> Option<String> {
    json_string_field(arguments, "patch")
}

fn partial_json_string_field(input: &str, field: &str) -> Option<String> {
    let key = format!("\"{field}\"");
    let key_start = input.find(&key)?;
    let after_key = &input[key_start + key.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    let mut chars = after_colon.chars();
    if chars.next()? != '"' {
        return None;
    }

    let mut out = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            match ch {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'u' => {}
                other => out.push(other),
            }
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_partial_patch_argument_while_json_streams() {
        let patch = partial_json_string_field(
            "{\"patch\":\"*** Begin Patch\\n*** Update File: src/lib.rs\\n@@\\n-old\\n+new",
            "patch",
        )
        .unwrap();

        assert!(patch.contains("*** Update File: src/lib.rs"));
        assert!(patch.contains("-old"));
        assert!(patch.contains("+new"));
    }
}
