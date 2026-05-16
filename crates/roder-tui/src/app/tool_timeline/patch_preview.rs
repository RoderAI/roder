use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

use super::super::Theme;
use super::ToolTimelineEntry;

const MAX_PATCH_PREVIEW_LINES: usize = 80;
const LINE_NUMBER_WIDTH: usize = 4;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct ToolDiffPreview {
    files: Vec<FileDiffPreview>,
}

impl ToolDiffPreview {
    pub(super) fn title(&self) -> String {
        let additions = self.additions();
        let deletions = self.deletions();
        if let [file] = self.files.as_slice() {
            return format!(
                "{} {} ({})",
                file.action.label(),
                file.path,
                change_counts(additions, deletions)
            );
        }

        format!(
            "Edited {} files ({})",
            self.files.len(),
            change_counts(additions, deletions)
        )
    }

    fn additions(&self) -> usize {
        self.files.iter().map(|file| file.additions).sum()
    }

    fn deletions(&self) -> usize {
        self.files.iter().map(|file| file.deletions).sum()
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct FileDiffPreview {
    action: FileDiffAction,
    path: String,
    additions: usize,
    deletions: usize,
    lines: Vec<DiffPreviewLine>,
}

impl FileDiffPreview {
    fn new(action: FileDiffAction, path: impl Into<String>) -> Self {
        Self {
            action,
            path: path.into(),
            additions: 0,
            deletions: 0,
            lines: Vec::new(),
        }
    }

    fn push(&mut self, kind: DiffPreviewLineKind, text: impl Into<String>) {
        match kind {
            DiffPreviewLineKind::Added => self.additions += 1,
            DiffPreviewLineKind::Removed => self.deletions += 1,
            DiffPreviewLineKind::Context | DiffPreviewLineKind::Hunk => {}
        }
        self.lines.push(DiffPreviewLine {
            kind,
            text: text.into(),
        });
    }

    fn is_empty(&self) -> bool {
        self.path.trim().is_empty()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum FileDiffAction {
    Added,
    Deleted,
    Edited,
    Wrote,
}

impl FileDiffAction {
    fn label(self) -> &'static str {
        match self {
            Self::Added => "Added",
            Self::Deleted => "Deleted",
            Self::Edited => "Edited",
            Self::Wrote => "Wrote",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct DiffPreviewLine {
    kind: DiffPreviewLineKind,
    text: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DiffPreviewLineKind {
    Context,
    Added,
    Removed,
    Hunk,
}

pub(super) fn tool_diff_preview(entry: &ToolTimelineEntry) -> Option<ToolDiffPreview> {
    match entry.name.as_str() {
        "apply_patch" => apply_patch_preview(&entry.arguments),
        "edit" => edit_preview(&entry.arguments),
        "multi_edit" => multi_edit_preview(&entry.arguments),
        "write_file" => write_file_preview(&entry.arguments),
        _ => None,
    }
}

pub(super) fn tool_diff_preview_lines(
    preview: &ToolDiffPreview,
    theme: Theme,
) -> Vec<Line<'static>> {
    render_diff_lines(preview, theme)
}

fn apply_patch_preview(arguments: &str) -> Option<ToolDiffPreview> {
    let patch = patch_argument(arguments)?;
    parse_patch_preview(patch.trim_end())
}

fn edit_preview(arguments: &str) -> Option<ToolDiffPreview> {
    let path = json_string_field(arguments, "path")?;
    let old = json_string_field(arguments, "old_string")?;
    let new = json_string_field(arguments, "new_string")?;
    Some(ToolDiffPreview {
        files: vec![edit_file_preview(path, &[(&old, &new)])],
    })
}

fn multi_edit_preview(arguments: &str) -> Option<ToolDiffPreview> {
    let value = serde_json::from_str::<Value>(arguments).ok()?;
    let path = value.get("path")?.as_str()?;
    let edits = value.get("edits")?.as_array()?;
    if edits.is_empty() {
        return None;
    }
    let mut pairs = Vec::with_capacity(edits.len());
    for edit in edits {
        pairs.push((
            edit.get("old_string")?.as_str()?,
            edit.get("new_string")?.as_str()?,
        ));
    }
    Some(ToolDiffPreview {
        files: vec![edit_file_preview(path.to_string(), &pairs)],
    })
}

fn write_file_preview(arguments: &str) -> Option<ToolDiffPreview> {
    let path = json_string_field(arguments, "path")?;
    let content = json_string_field(arguments, "content")?;
    let mut file = FileDiffPreview::new(FileDiffAction::Wrote, path);
    file.push(DiffPreviewLineKind::Hunk, String::new());
    push_added_lines(&mut file, &content);
    Some(ToolDiffPreview { files: vec![file] })
}

fn render_diff_lines(preview: &ToolDiffPreview, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut rendered = 0usize;
    for file in &preview.files {
        if preview.files.len() > 1 {
            if rendered >= MAX_PATCH_PREVIEW_LINES {
                break;
            }
            lines.push(file_header_line(file, theme));
            rendered += 1;
        }

        let mut before_line = 1usize;
        let mut after_line = 1usize;
        for diff_line in &file.lines {
            if rendered >= MAX_PATCH_PREVIEW_LINES {
                break;
            }
            lines.push(diff_line_row(
                diff_line,
                &mut before_line,
                &mut after_line,
                theme,
            ));
            rendered += 1;
        }
        if rendered >= MAX_PATCH_PREVIEW_LINES {
            break;
        }
    }

    if preview
        .files
        .iter()
        .map(|file| file.lines.len() + usize::from(preview.files.len() > 1))
        .sum::<usize>()
        > MAX_PATCH_PREVIEW_LINES
    {
        lines.push(Line::from(vec![
            Span::styled("  ".to_string(), theme.diff_line_number()),
            Span::styled(
                "diff preview truncated in timeline",
                theme.muted().add_modifier(Modifier::ITALIC),
            ),
        ]));
    }
    lines
}

fn file_header_line(file: &FileDiffPreview, theme: Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled("  ".to_string(), theme.subtle()),
        Span::styled(
            format!(
                "{} {} ({})",
                file.action.label(),
                file.path,
                change_counts(file.additions, file.deletions)
            ),
            theme.tool(),
        ),
    ])
}

fn diff_line_row(
    line: &DiffPreviewLine,
    before_line: &mut usize,
    after_line: &mut usize,
    theme: Theme,
) -> Line<'static> {
    match line.kind {
        DiffPreviewLineKind::Hunk => Line::from(vec![
            Span::styled("     ".to_string(), theme.diff_line_number()),
            Span::styled("⋮".to_string(), theme.diff_line_number()),
            Span::styled(hunk_text(&line.text), theme.muted()),
        ]),
        DiffPreviewLineKind::Context => {
            let number = *before_line;
            *before_line += 1;
            *after_line += 1;
            diff_content_line(
                number,
                " ",
                &line.text,
                theme.diff_line_number(),
                theme.text(),
            )
        }
        DiffPreviewLineKind::Added => {
            let number = *after_line;
            *after_line += 1;
            let style = theme.diff_added();
            diff_content_line(number, "+", &line.text, style, style)
        }
        DiffPreviewLineKind::Removed => {
            let number = *before_line;
            *before_line += 1;
            let style = theme.diff_removed();
            diff_content_line(number, "-", &line.text, style, style)
        }
    }
}

fn diff_content_line(
    number: usize,
    marker: &'static str,
    text: &str,
    gutter_style: Style,
    body_style: Style,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {:>width$} ", number, width = LINE_NUMBER_WIDTH),
            gutter_style,
        ),
        Span::styled(marker.to_string(), body_style),
        Span::styled(text.to_string(), body_style),
    ])
}

fn hunk_text(text: &str) -> String {
    if text.trim().is_empty() {
        String::new()
    } else {
        format!(" {}", text.trim())
    }
}

fn edit_file_preview(path: String, edits: &[(&str, &str)]) -> FileDiffPreview {
    let mut file = FileDiffPreview::new(FileDiffAction::Edited, path);
    for (old, new) in edits {
        file.push(DiffPreviewLineKind::Hunk, String::new());
        push_removed_lines(&mut file, old);
        push_added_lines(&mut file, new);
    }
    file
}

fn push_removed_lines(file: &mut FileDiffPreview, text: &str) {
    if text.is_empty() {
        return;
    }
    for line in text.lines() {
        file.push(DiffPreviewLineKind::Removed, line);
    }
}

fn push_added_lines(file: &mut FileDiffPreview, text: &str) {
    if text.is_empty() {
        file.push(DiffPreviewLineKind::Added, String::new());
        return;
    }
    for line in text.lines() {
        file.push(DiffPreviewLineKind::Added, line);
    }
}

fn parse_patch_preview(patch: &str) -> Option<ToolDiffPreview> {
    if patch.trim().is_empty() {
        return None;
    }

    let mut files = Vec::new();
    let mut current: Option<FileDiffPreview> = None;

    for line in patch.lines() {
        if line == "*** Begin Patch" || line == "*** End Patch" {
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Add File: ") {
            push_current_file(&mut files, &mut current);
            current = Some(FileDiffPreview::new(FileDiffAction::Added, path));
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            push_current_file(&mut files, &mut current);
            current = Some(FileDiffPreview::new(FileDiffAction::Deleted, path));
            continue;
        }
        if let Some(path) = line
            .strip_prefix("*** Update File: ")
            .or_else(|| line.strip_prefix("*** Edit File: "))
        {
            push_current_file(&mut files, &mut current);
            current = Some(FileDiffPreview::new(FileDiffAction::Edited, path));
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Write File: ") {
            push_current_file(&mut files, &mut current);
            current = Some(FileDiffPreview::new(FileDiffAction::Wrote, path));
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Move to: ") {
            if let Some(file) = current.as_mut() {
                file.path = format!("{} -> {path}", file.path);
            }
            continue;
        }

        let Some(file) = current.as_mut() else {
            continue;
        };
        if let Some(hunk) = line.strip_prefix("@@") {
            file.push(DiffPreviewLineKind::Hunk, hunk.trim().to_string());
        } else if line.starts_with('+') && !line.starts_with("+++") {
            file.push(DiffPreviewLineKind::Added, &line[1..]);
        } else if line.starts_with('-') && !line.starts_with("---") {
            file.push(DiffPreviewLineKind::Removed, &line[1..]);
        } else if let Some(context) = line.strip_prefix(' ') {
            file.push(DiffPreviewLineKind::Context, context);
        }
    }
    push_current_file(&mut files, &mut current);

    if files.is_empty() {
        None
    } else {
        Some(ToolDiffPreview { files })
    }
}

fn push_current_file(files: &mut Vec<FileDiffPreview>, current: &mut Option<FileDiffPreview>) {
    if let Some(file) = current.take()
        && !file.is_empty()
    {
        files.push(file);
    }
}

fn change_counts(additions: usize, deletions: usize) -> String {
    format!("+{additions} -{deletions}")
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

    #[test]
    fn parses_apply_patch_into_file_preview() {
        let preview = parse_patch_preview(
            "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n old\n-new\n+newer\n*** End Patch",
        )
        .unwrap();

        assert_eq!(preview.title(), "Edited src/lib.rs (+1 -1)");
        assert_eq!(preview.files[0].lines.len(), 4);
    }

    #[test]
    fn parses_apply_patch_file_header_before_hunk_streams() {
        let preview = parse_patch_preview("*** Begin Patch\n*** Update File: src/lib.rs").unwrap();

        assert_eq!(preview.title(), "Edited src/lib.rs (+0 -0)");
        assert!(preview.files[0].lines.is_empty());
    }
}
