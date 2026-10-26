use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::super::Theme;
use crate::transcript::links::{self, MarkdownLink};

pub(super) fn markdown_lines(body: &str, base_style: Style, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;

    for raw_line in body.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            lines.push(Line::from(vec![
                Span::styled("    ".to_string(), theme.subtle()),
                Span::styled(line.to_string(), base_style.add_modifier(Modifier::BOLD)),
            ]));
            continue;
        }

        if trimmed.is_empty() {
            lines.push(Line::raw(""));
            continue;
        }

        if let Some(heading) = heading_text(trimmed) {
            lines.push(Line::from(inline_spans(
                heading,
                base_style.add_modifier(Modifier::BOLD),
                theme,
            )));
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
            continue;
        }

        if let Some(rest) = unordered_list_item(trimmed) {
            let mut spans = vec![Span::styled("- ".to_string(), theme.subtle())];
            spans.extend(inline_spans(rest, base_style, theme));
            lines.push(Line::from(spans));
            continue;
        }

        if let Some((marker, rest)) = ordered_list_item(trimmed) {
            let mut spans = vec![Span::styled(marker, theme.subtle())];
            spans.extend(inline_spans(rest, base_style, theme));
            lines.push(Line::from(spans));
            continue;
        }

        lines.push(Line::from(inline_spans(trimmed, base_style, theme)));
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
