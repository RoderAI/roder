use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::super::Theme;
use crate::transcript::links::{self, MarkdownLink};

pub(super) fn markdown_lines(
    body: &str,
    base_style: Style,
    theme: Theme,
    max_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let raw_lines: Vec<&str> = body.lines().collect();
    let mut index = 0;

    while index < raw_lines.len() {
        let line = raw_lines[index].trim_end();
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            index += 1;
            continue;
        }

        if in_code_block {
            lines.push(Line::from(vec![
                Span::styled("    ".to_string(), theme.subtle()),
                Span::styled(line.to_string(), base_style.add_modifier(Modifier::BOLD)),
            ]));
            index += 1;
            continue;
        }

        if let Some((consumed, table_lines)) =
            take_markdown_table(&raw_lines[index..], base_style, theme, max_width)
        {
            lines.extend(table_lines);
            index += consumed;
            continue;
        }

        if trimmed.is_empty() {
            lines.push(Line::raw(""));
            index += 1;
            continue;
        }

        if let Some(heading) = heading_text(trimmed) {
            lines.push(Line::from(inline_spans(
                heading,
                base_style.add_modifier(Modifier::BOLD),
                theme,
            )));
            index += 1;
            continue;
        }

        if let Some(quote) = trimmed.strip_prefix('>') {
            let quote = quote.trim_start();
            let mut spans = vec![Span::styled("> ".to_string(), theme.subtle())];
            spans.extend(inline_spans(
                quote,
                base_style.add_modifier(Modifier::ITALIC),
                theme,
            ));
            lines.push(Line::from(spans));
            index += 1;
            continue;
        }

        if let Some(rest) = unordered_list_item(trimmed) {
            let mut spans = vec![Span::styled("- ".to_string(), theme.subtle())];
            spans.extend(inline_spans(rest, base_style, theme));
            lines.push(Line::from(spans));
            index += 1;
            continue;
        }

        if let Some((marker, rest)) = ordered_list_item(trimmed) {
            let mut spans = vec![Span::styled(marker, theme.subtle())];
            spans.extend(inline_spans(rest, base_style, theme));
            lines.push(Line::from(spans));
            index += 1;
            continue;
        }

        lines.push(Line::from(inline_spans(trimmed, base_style, theme)));
        index += 1;
    }

    if body.ends_with('\n') {
        lines.push(Line::raw(""));
    }

    lines
}

fn heading_text(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = line.get(hashes..)?;
    rest.strip_prefix(' ').map(str::trim)
}

fn unordered_list_item(line: &str) -> Option<&str> {
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|marker| line.strip_prefix(marker).map(str::trim_start))
}

fn ordered_list_item(line: &str) -> Option<(String, &str)> {
    let dot = line.find('.')?;
    if dot == 0 || !line[..dot].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let rest = line.get(dot + 1..)?.strip_prefix(' ')?;
    Some((format!("{}. ", &line[..dot]), rest.trim_start()))
}

fn inline_spans(text: &str, base_style: Style, theme: Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;

    while !rest.is_empty() {
        let Some((pos, mark)) = next_mark(rest) else {
            spans.push(Span::styled(rest.to_string(), base_style));
            break;
        };

        if pos > 0 {
            spans.push(Span::styled(rest[..pos].to_string(), base_style));
            rest = &rest[pos..];
            continue;
        }

        match mark {
            InlineMark::Token(token) => {
                if !consume_emphasis(&mut spans, &mut rest, token, base_style) {
                    spans.push(Span::styled(token.to_string(), base_style));
                    rest = &rest[token.len()..];
                }
            }
            InlineMark::Link(link) => {
                push_link_spans(&mut spans, &link, base_style, theme);
                rest = &rest[link.consumed..];
            }
        }
    }

    spans
}

enum InlineMark {
    Token(&'static str),
    Link(MarkdownLink),
}

fn next_mark(text: &str) -> Option<(usize, InlineMark)> {
    let mut best: Option<(usize, InlineMark)> = None;
    for token in ["**", "__", "`", "*", "_"] {
        if let Some(index) = text.find(token) {
            if best
                .as_ref()
                .is_none_or(|(best_index, _)| index < *best_index)
            {
                best = Some((index, InlineMark::Token(token)));
            }
        }
    }
    if let Some(index) = links::find_link_start(text) {
        if let Some(link) = links::parse_leading_markdown_link(&text[index..]) {
            if best
                .as_ref()
                .is_none_or(|(best_index, _)| index < *best_index)
            {
                best = Some((index, InlineMark::Link(link)));
            }
        }
    }
    best
}

fn consume_emphasis(
    spans: &mut Vec<Span<'static>>,
    rest: &mut &str,
    token: &str,
    base_style: Style,
) -> bool {
    let modifier = match token {
        "**" | "__" | "`" => Modifier::BOLD,
        "*" | "_" => Modifier::ITALIC,
        _ => return false,
    };
    let Some(end) = rest[token.len()..].find(token) else {
        return false;
    };
    let content = &rest[token.len()..token.len() + end];
    spans.push(Span::styled(
        content.to_string(),
        base_style.add_modifier(modifier),
    ));
    *rest = &rest[token.len() + end + token.len()..];
    true
}

fn push_link_spans(
    spans: &mut Vec<Span<'static>>,
    link: &MarkdownLink,
    base_style: Style,
    theme: Theme,
) {
    let visible = if link.label.is_empty() {
        link.destination.as_str()
    } else {
        link.label.as_str()
    };
    let inner = if link.label.is_empty() {
        vec![Span::styled(visible.to_string(), base_style)]
    } else {
        inline_spans(visible, base_style, theme)
    };
    if inner.is_empty() {
        spans.push(Span::styled(
            visible.to_string(),
            base_style.patch(theme.link()),
        ));
        return;
    }
    for span in inner {
        spans.push(Span::styled(span.content, span.style.patch(theme.link())));
    }
}

#[derive(Clone, Copy)]
enum ColumnAlign {
    Left,
    Right,
    Center,
}

/// A GFM pipe table is a header row, a delimiter row, then body rows. Render it
/// as an aligned grid when it fits, otherwise as labeled rows so wrapping does
/// not smash the columns together.
fn take_markdown_table(
    raw_lines: &[&str],
    base_style: Style,
    theme: Theme,
    max_width: usize,
) -> Option<(usize, Vec<Line<'static>>)> {
    if raw_lines.len() < 2 {
        return None;
    }
    let header = split_table_cells(raw_lines[0].trim())?;
    let aligns = separator_alignments(raw_lines[1].trim())?;
    if header.is_empty() || header.len() != aligns.len() {
        return None;
    }

    let mut rows = Vec::new();
    let mut consumed = 2;
    while consumed < raw_lines.len() {
        let trimmed = raw_lines[consumed].trim();
        if trimmed.is_empty()
            || trimmed.starts_with("```")
            || trimmed.starts_with("~~~")
            || separator_alignments(trimmed).is_some()
        {
            break;
        }
        let Some(cells) = split_table_cells(trimmed) else {
            break;
        };
        rows.push(cells);
        consumed += 1;
    }

    Some((
        consumed,
        render_markdown_table(&header, &aligns, &rows, base_style, theme, max_width),
    ))
}

fn split_table_cells(line: &str) -> Option<Vec<String>> {
    if !line.contains('|') {
        return None;
    }
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = line.chars().peekable();
    let mut in_code = false;
    while let Some(ch) = chars.next() {
        if ch == '`' {
            in_code = !in_code;
            cell.push(ch);
            continue;
        }
        if ch == '\\' && !in_code {
            if chars.peek() == Some(&'|') {
                chars.next();
                cell.push('|');
            } else if let Some(next) = chars.next() {
                cell.push('\\');
                cell.push(next);
            } else {
                cell.push('\\');
            }
            continue;
        }
        if ch == '|' && !in_code {
            cells.push(cell.trim().to_string());
            cell.clear();
            continue;
        }
        cell.push(ch);
    }
    cells.push(cell.trim().to_string());
    if cells.first().is_some_and(String::is_empty) {
        cells.remove(0);
    }
    if cells.last().is_some_and(String::is_empty) {
        cells.pop();
    }
    (!cells.is_empty()).then_some(cells)
}

fn separator_alignments(line: &str) -> Option<Vec<ColumnAlign>> {
    let cells = split_table_cells(line)?;
    cells
        .iter()
        .map(|cell| align_of_separator_cell(cell))
        .collect()
}

fn align_of_separator_cell(cell: &str) -> Option<ColumnAlign> {
    let cell = cell.trim();
    if cell.is_empty() {
        return None;
    }
    let left = cell.starts_with(':');
    let right = cell.ends_with(':');
    let dashes = cell.trim_matches(':');
    if dashes.is_empty() || !dashes.bytes().all(|byte| byte == b'-') {
        return None;
    }
    Some(match (left, right) {
        (true, true) => ColumnAlign::Center,
        (false, true) => ColumnAlign::Right,
        _ => ColumnAlign::Left,
    })
}

fn render_markdown_table(
    header: &[String],
    aligns: &[ColumnAlign],
    rows: &[Vec<String>],
    base_style: Style,
    theme: Theme,
    max_width: usize,
) -> Vec<Line<'static>> {
    let columns = header.len();
    let header_style = base_style.add_modifier(Modifier::BOLD);
    let header_plain: Vec<Vec<(String, Style)>> = header
        .iter()
        .map(|cell| styled_chars(&inline_spans(cell, header_style, theme)))
        .collect();
    let body_plain: Vec<Vec<Vec<(String, Style)>>> = rows
        .iter()
        .map(|row| {
            (0..columns)
                .map(|column| {
                    let cell = row.get(column).map(String::as_str).unwrap_or("");
                    styled_chars(&inline_spans(cell, base_style, theme))
                })
                .collect()
        })
        .collect();
    let preferred: Vec<usize> = (0..columns)
        .map(|column| {
            let header_width = styled_width(&header_plain[column]).max(1);
            let body_width = body_plain
                .iter()
                .map(|row| styled_width(&row[column]))
                .max()
                .unwrap_or(0);
            header_width.max(body_width).max(1)
        })
        .collect();
    let floors: Vec<usize> = (0..columns)
        .map(|column| {
            let header_word = longest_word(&header_plain[column]);
            let body_word = body_plain
                .iter()
                .map(|row| longest_word(&row[column]))
                .max()
                .unwrap_or(1);
            header_word.max(body_word).max(1)
        })
        .collect();

    if let Some(widths) = fit_column_widths(&preferred, &floors, max_width) {
        render_table_grid(
            &header_plain,
            &body_plain,
            aligns,
            &widths,
            base_style,
            theme,
        )
    } else {
        render_table_stacked(header, rows, base_style, theme, max_width)
    }
}

fn longest_word(chars: &[(String, Style)]) -> usize {
    chars
        .split(|(ch, _)| ch == " ")
        .map(styled_width)
        .max()
        .unwrap_or(1)
        .max(1)
}

fn fit_column_widths(
    preferred: &[usize],
    floors: &[usize],
    max_width: usize,
) -> Option<Vec<usize>> {
    let columns = preferred.len();
    if columns == 0 {
        return Some(Vec::new());
    }
    if max_width == 0 {
        return Some(preferred.to_vec());
    }
    let overhead = 3 * columns.saturating_sub(1);
    let budget = max_width.saturating_sub(overhead);
    if floors.iter().sum::<usize>() > budget {
        return None;
    }
    if preferred.iter().sum::<usize>() <= budget {
        return Some(preferred.to_vec());
    }

    let mut widths = preferred.to_vec();
    let mut excess = widths.iter().sum::<usize>() - budget;
    while excess > 0 {
        let shrinkable: Vec<usize> = (0..columns)
            .filter(|&index| widths[index] > floors[index])
            .collect();
        if shrinkable.is_empty() {
            return None;
        }
        let share = (excess / shrinkable.len()).max(1);
        let mut cut_any = false;
        for index in shrinkable {
            if excess == 0 {
                break;
            }
            let room = widths[index] - floors[index];
            let cut = share.min(room).min(excess);
            if cut == 0 {
                continue;
            }
            widths[index] -= cut;
            excess -= cut;
            cut_any = true;
        }
        if !cut_any {
            return None;
        }
    }
    Some(widths)
}

fn render_table_grid(
    header: &[Vec<(String, Style)>],
    rows: &[Vec<Vec<(String, Style)>>],
    aligns: &[ColumnAlign],
    widths: &[usize],
    base_style: Style,
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    push_wrapped_grid_row(&mut lines, header, aligns, widths, base_style, theme);
    lines.push(grid_rule(widths, theme));
    for row in rows {
        push_wrapped_grid_row(&mut lines, row, aligns, widths, base_style, theme);
    }
    lines
}

fn push_wrapped_grid_row(
    lines: &mut Vec<Line<'static>>,
    cells: &[Vec<(String, Style)>],
    aligns: &[ColumnAlign],
    widths: &[usize],
    base_style: Style,
    theme: Theme,
) {
    let wrapped: Vec<Vec<Vec<(String, Style)>>> = cells
        .iter()
        .zip(widths)
        .map(|(cell, width)| wrap_styled(cell, *width))
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);
    for line_index in 0..height {
        let line_cells: Vec<Vec<(String, Style)>> = wrapped
            .iter()
            .map(|cell| cell.get(line_index).cloned().unwrap_or_default())
            .collect();
        lines.push(grid_row(&line_cells, aligns, widths, base_style, theme));
    }
}

fn grid_row(
    cells: &[Vec<(String, Style)>],
    aligns: &[ColumnAlign],
    widths: &[usize],
    base_style: Style,
    theme: Theme,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" ".to_string(), base_style));
            spans.push(Span::styled("│".to_string(), theme.border()));
            spans.push(Span::styled(" ".to_string(), base_style));
        }
        let cell = cells.get(index).map(Vec::as_slice).unwrap_or(&[]);
        let align = aligns.get(index).copied().unwrap_or(ColumnAlign::Left);
        spans.extend(chars_to_spans(&pad_styled(cell, *width, align, base_style)));
    }
    Line::from(spans)
}

fn grid_rule(widths: &[usize], theme: Theme) -> Line<'static> {
    let mut rule = String::new();
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            rule.push_str("─┼─");
        }
        rule.push_str(&"─".repeat(*width));
    }
    Line::from(Span::styled(rule, theme.border()))
}

fn render_table_stacked(
    header: &[String],
    rows: &[Vec<String>],
    base_style: Style,
    theme: Theme,
    max_width: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return vec![Line::from(inline_spans(
            &header.join("  "),
            base_style.add_modifier(Modifier::BOLD),
            theme,
        ))];
    }
    let mut lines = Vec::new();
    for (record_index, record) in rows.iter().enumerate() {
        if record_index > 0 {
            lines.push(Line::raw(""));
        }
        for (column, label) in header.iter().enumerate() {
            let value = record.get(column).map(String::as_str).unwrap_or("");
            lines.extend(stacked_field(label, value, base_style, theme, max_width));
        }
    }
    lines
}

fn stacked_field(
    label: &str,
    value: &str,
    base_style: Style,
    theme: Theme,
    max_width: usize,
) -> Vec<Line<'static>> {
    let label_spans = inline_spans(label, theme.subtle().add_modifier(Modifier::BOLD), theme);
    let value_spans = inline_spans(value, base_style, theme);
    let label_width = spans_char_width(&label_spans);
    let value_width = spans_char_width(&value_spans);
    let inline_width = label_width + 2 + value_width;
    if max_width == 0 || inline_width <= max_width.max(1) {
        let mut spans = label_spans;
        spans.push(Span::styled("  ".to_string(), base_style));
        spans.extend(value_spans);
        return vec![Line::from(spans)];
    }

    let mut lines: Vec<_> = wrap_styled(&styled_chars(&label_spans), max_width.max(1))
        .iter()
        .map(|line| Line::from(chars_to_spans(line)))
        .collect();
    let indent = max_width.saturating_sub(2).min(2);
    let value_budget = max_width.saturating_sub(indent).max(1);
    for wrapped in wrap_styled(&styled_chars(&value_spans), value_budget) {
        let mut spans = vec![Span::styled(" ".repeat(indent), theme.subtle())];
        spans.extend(chars_to_spans(&wrapped));
        lines.push(Line::from(spans));
    }
    lines
}

fn styled_chars(spans: &[Span<'_>]) -> Vec<(String, Style)> {
    spans
        .iter()
        .flat_map(|span| {
            span.styled_graphemes(Style::default())
                .map(|grapheme| (grapheme.symbol.to_string(), grapheme.style))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn spans_char_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

fn styled_width(chars: &[(String, Style)]) -> usize {
    chars
        .iter()
        .map(|(symbol, _)| Span::raw(symbol.as_str()).width())
        .sum()
}

fn wrap_styled(chars: &[(String, Style)], width: usize) -> Vec<Vec<(String, Style)>> {
    if width == 0 || chars.is_empty() {
        return vec![if chars.is_empty() {
            Vec::new()
        } else {
            chars.to_vec()
        }];
    }
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut last_break = None;
    for item in chars {
        current.push(item.clone());
        if item.0 == " " {
            last_break = Some(current.len());
        }
        if styled_width(&current) <= width || current.len() == 1 {
            continue;
        }
        if let Some(break_at) = last_break.filter(|at| *at > 0 && *at < current.len()) {
            let next = current.split_off(break_at);
            if current.last().is_some_and(|(ch, _)| ch == " ") {
                current.pop();
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            current = next;
            trim_leading_spaces(&mut current);
            last_break = current
                .iter()
                .rposition(|(ch, _)| ch == " ")
                .map(|index| index + 1);
        } else {
            let overflow = current.pop().unwrap();
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            current = vec![overflow];
            trim_leading_spaces(&mut current);
            last_break = None;
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn trim_leading_spaces(chars: &mut Vec<(String, Style)>) {
    while chars.first().is_some_and(|(ch, _)| ch == " ") {
        chars.remove(0);
    }
}

fn pad_styled(
    chars: &[(String, Style)],
    width: usize,
    align: ColumnAlign,
    pad_style: Style,
) -> Vec<(String, Style)> {
    let gap = width.saturating_sub(styled_width(chars));
    let (left, right) = match align {
        ColumnAlign::Left => (0, gap),
        ColumnAlign::Right => (gap, 0),
        ColumnAlign::Center => (gap / 2, gap - gap / 2),
    };
    let mut padded = Vec::with_capacity(width);
    padded.extend(std::iter::repeat_n((" ".to_string(), pad_style), left));
    padded.extend(chars.iter().cloned());
    padded.extend(std::iter::repeat_n((" ".to_string(), pad_style), right));
    padded
}

fn chars_to_spans(chars: &[(String, Style)]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut style = Style::default();
    for (index, (ch, ch_style)) in chars.iter().enumerate() {
        if index == 0 {
            style = *ch_style;
        } else if *ch_style != style {
            spans.push(Span::styled(std::mem::take(&mut current), style));
            style = *ch_style;
        }
        current.push_str(ch);
    }
    if !current.is_empty() {
        spans.push(Span::styled(current, style));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::super::super::Theme;
    use super::*;

    fn visible(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn pipe_table_renders_aligned_grid_without_delimiter_row() {
        let theme = Theme::for_dark_background(true);
        let lines = markdown_lines(
            "| Name | Role |\n| --- | --- |\n| **Ada** | Engineer |\n| Grace | Math |\n",
            theme.text(),
            theme,
            80,
        );
        let text = visible(&lines);
        let bars: Vec<_> = text.iter().filter_map(|line| line.find('│')).collect();
        assert!(
            bars.len() >= 3 && bars.windows(2).all(|pair| pair[0] == pair[1]),
            "columns should share a vertical rule, got {text:?}"
        );
        assert!(
            text.iter().any(|line| line.contains('┼')),
            "header rule should join columns, got {text:?}"
        );
        assert!(
            text.iter()
                .any(|line| line.contains("Ada") && line.contains("Engineer")),
            "bold markers should not widen the name column, got {text:?}"
        );
        assert!(
            text.iter()
                .any(|line| line.contains("Grace") && line.contains("Math")),
            "body columns should line up, got {text:?}"
        );
        assert!(text.iter().all(|line| !line.contains("|---")));
    }

    #[test]
    fn pipe_table_without_outer_pipes_still_aligns() {
        let theme = Theme::for_dark_background(true);
        let lines = markdown_lines(
            "Name | Role\n--- | ---\nAda | Engineer\n",
            theme.text(),
            theme,
            40,
        );
        let text = visible(&lines);
        assert!(
            text.iter().any(|line| line.contains("Ada")
                && line.contains("Engineer")
                && line.contains('│')),
            "tables without outer pipes should still grid, got {text:?}"
        );
    }

    #[test]
    fn pipe_table_honors_right_alignment() {
        let theme = Theme::for_dark_background(true);
        let lines = markdown_lines(
            "| Item | Qty |\n| --- | ---: |\n| Ada | 12 |\n",
            theme.text(),
            theme,
            40,
        );
        let text = visible(&lines);
        assert!(
            text.iter().any(|line| line.ends_with("│  12")),
            "numeric column should be right-aligned, got {text:?}"
        );
    }

    #[test]
    fn narrow_table_falls_back_to_labeled_rows() {
        let theme = Theme::for_dark_background(true);
        let lines = markdown_lines(
            "| Name | Role |\n| --- | --- |\n| Ada Lovelace | First programmer |\n",
            theme.text(),
            theme,
            12,
        );
        let text = visible(&lines);
        assert!(
            text.iter().any(|line| line.contains("Name"))
                && text.iter().any(|line| line.contains("Lovelace")),
            "narrow tables should keep each field readable, got {text:?}"
        );
        assert!(text.iter().all(|line| !line.contains('│')));
    }

    #[test]
    fn prose_with_a_pipe_is_not_a_table() {
        let theme = Theme::for_dark_background(true);
        let lines = markdown_lines("Use a | b in prose.\nNext line.\n", theme.text(), theme, 80);
        let text = visible(&lines);
        assert!(text.iter().any(|line| line.contains("a | b")));
        assert!(text.iter().all(|line| !line.contains('│')));
    }

    #[test]
    fn fenced_pipe_table_stays_literal() {
        let theme = Theme::for_dark_background(true);
        let lines = markdown_lines(
            "```\n| A | B |\n| - | - |\n| 1 | 2 |\n```\n",
            theme.text(),
            theme,
            80,
        );
        let text = visible(&lines);
        assert!(text.iter().any(|line| line.contains("| A | B |")));
        assert!(text.iter().all(|line| !line.contains('│')));
    }

    #[test]
    fn unicode_table_columns_use_terminal_cell_widths() {
        let theme = Theme::for_dark_background(true);
        for value in ["\u{540d}\u{79f0}", "\u{1f469}\u{200d}\u{1f4bb}", "e\u{301}"] {
            let body = format!("| K | V |\n| - | - |\n| **{value}** | x |");
            let lines = markdown_lines(&body, theme.text(), theme, 12);
            let text = visible(&lines);
            let positions: Vec<_> = text
                .iter()
                .filter_map(|line| {
                    line.find('│')
                        .map(|index| Span::raw(&line[..index]).width())
                })
                .collect();
            assert_eq!(positions.len(), 2, "{text:?}");
            assert_eq!(positions[0], positions[1], "{text:?}");
            assert!(lines.iter().all(|line| line.width() <= 12), "{text:?}");
            assert!(lines.iter().flat_map(|line| &line.spans).any(|span| {
                span.content.contains(value) && span.style.add_modifier.contains(Modifier::BOLD)
            }));
        }
    }

    #[test]
    fn combining_graphemes_do_not_force_a_wider_grid() {
        let theme = Theme::for_dark_background(true);
        let lines = markdown_lines(
            "| K | V |\n| - | - |\n| e\u{301} | x |",
            theme.text(),
            theme,
            5,
        );
        let text = visible(&lines);
        assert!(text.iter().any(|line| line.contains('│')), "{text:?}");
        assert!(lines.iter().all(|line| line.width() <= 5), "{text:?}");
        assert!(text.iter().any(|line| line.contains("e\u{301}")));
    }

    #[test]
    fn narrow_unicode_table_stacks_without_overflowing() {
        let theme = Theme::for_dark_background(true);
        let lines = markdown_lines(
            "| K | V |\n| - | - |\n| \u{540d}\u{79f0} | x |",
            theme.text(),
            theme,
            6,
        );
        let text = visible(&lines);
        assert!(text.iter().all(|line| !line.contains('│')), "{text:?}");
        assert!(lines.iter().all(|line| line.width() <= 6), "{text:?}");
        assert!(text.iter().any(|line| line.contains("\u{540d}\u{79f0}")));
    }

    #[test]
    fn table_wrapping_preserves_graphemes_and_style() {
        let style = Style::default().add_modifier(Modifier::BOLD);
        for (glyph, width) in [
            ("\u{540d}", 2),
            ("\u{1f469}\u{200d}\u{1f4bb}", 2),
            ("e\u{301}", 1),
        ] {
            let content = glyph.repeat(3);
            let chars = styled_chars(&[Span::styled(content, style)]);
            let lines = wrap_styled(&chars, width);
            assert_eq!(lines.len(), 3, "{lines:?}");
            for line in lines {
                assert_eq!(styled_width(&line), width);
                let spans = chars_to_spans(&line);
                assert_eq!(spans.len(), 1);
                assert_eq!(spans[0].content, glyph);
                assert_eq!(spans[0].style, style);
            }
        }
    }
}
