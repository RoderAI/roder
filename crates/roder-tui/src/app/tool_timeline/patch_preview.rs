#![allow(dead_code)]
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

use super::super::Theme;
use super::ToolTimelineEntry;
use crate::syntax::{SyntaxTheme, highlight_code, language_for_path};

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
    width: u16,
) -> Vec<Line<'static>> {
    render_diff_lines(preview, theme, width)
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

fn render_diff_lines(preview: &ToolDiffPreview, theme: Theme, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut rendered = 0usize;
    for file in &preview.files {
        if rendered >= MAX_PATCH_PREVIEW_LINES {
            break;
        }
        lines.push(file_header_line(file, theme, width));
        rendered += 1;
        let language = language_for_path(std::path::Path::new(&file.path));

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
                language,
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
        .map(|file| file.lines.len() + 1)
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

fn file_header_line(file: &FileDiffPreview, theme: Theme, width: u16) -> Line<'static> {
    // Left-aligned: "  ▶ path"
    // Right-aligned: "+2 -1"
    let left_text = format!("  ▶ {}", file.path);
    let left_len = left_text.chars().count();

    // Format right-aligned count
    let add_str = format!("+{}", file.additions);
    let del_str = format!("-{}", file.deletions);
    let right_len = add_str.chars().count() + 1 + del_str.chars().count(); // "+2 -1"

    let pad_len = usize::from(width).saturating_sub(left_len + right_len);

    Line::from(vec![
        Span::styled("  ▶ ", theme.subtle()),
        Span::styled(
            file.path.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".repeat(pad_len), Style::default()),
        Span::styled(add_str, Style::default().fg(theme.diff_added)),
        Span::styled(" ", Style::default()),
        Span::styled(del_str, Style::default().fg(theme.diff_removed)),
    ])
}

fn diff_line_row(
    line: &DiffPreviewLine,
    before_line: &mut usize,
    after_line: &mut usize,
    theme: Theme,
    language: Option<crate::syntax::SyntaxLanguage>,
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
            let style = theme.text();
            diff_content_line(
                number,
                " ",
                &line.text,
                Style::default().fg(theme.diff_line_number),
                style,
                theme,
                language,
            )
        }
        DiffPreviewLineKind::Added => {
            let number = *after_line;
            *after_line += 1;
            let style = Style::default().fg(theme.text).bg(theme.diff_added_bg);
            let gutter_style = Style::default()
                .fg(theme.diff_line_number)
                .bg(theme.diff_added_bg);
            diff_content_line(
                number,
                "+",
                &line.text,
                gutter_style,
                style,
                theme,
                language,
            )
        }
        DiffPreviewLineKind::Removed => {
            let number = *before_line;
            *before_line += 1;
            let style = Style::default().fg(theme.text).bg(theme.diff_removed_bg);
            let gutter_style = Style::default()
                .fg(theme.diff_line_number)
                .bg(theme.diff_removed_bg);
            diff_content_line(
                number,
                "-",
                &line.text,
                gutter_style,
                style,
                theme,
                language,
            )
        }
    }
}

fn diff_content_line(
    number: usize,
    marker: &'static str,
    text: &str,
    gutter_style: Style,
    body_style: Style,
    theme: Theme,
    language: Option<crate::syntax::SyntaxLanguage>,
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            format!(" {:>width$} ", number, width = LINE_NUMBER_WIDTH),
            gutter_style,
        ),
        Span::styled(marker.to_string(), body_style),
    ];
    spans.extend(highlight_code(
        text,
        language,
        syntax_theme_from_style(body_style, theme),
    ));
    Line::from(spans)
}

fn syntax_theme_from_style(style: Style, theme: Theme) -> SyntaxTheme {
    let base = theme.text;
    SyntaxTheme {
        base,
        keyword: theme.accent,
        string: theme.commentary,
        number: theme.commentary,
        comment: theme.muted,
        ty: theme.accent_soft,
        function: base,
        mac: theme.commentary,
        bg: style.bg,
    }
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
