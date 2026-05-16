use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::super::Theme;

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
            )));
            continue;
        }

        if let Some(quote) = trimmed.strip_prefix('>') {
            let quote = quote.trim_start();
            let mut spans = vec![Span::styled("> ".to_string(), theme.subtle())];
            spans.extend(inline_spans(
                quote,
                base_style.add_modifier(Modifier::ITALIC),
            ));
            lines.push(Line::from(spans));
            continue;
        }

        if let Some(rest) = unordered_list_item(trimmed) {
            let mut spans = vec![Span::styled("- ".to_string(), theme.subtle())];
            spans.extend(inline_spans(rest, base_style));
            lines.push(Line::from(spans));
            continue;
        }

        if let Some((marker, rest)) = ordered_list_item(trimmed) {
            let mut spans = vec![Span::styled(marker, theme.subtle())];
            spans.extend(inline_spans(rest, base_style));
            lines.push(Line::from(spans));
            continue;
        }

        lines.push(Line::from(inline_spans(trimmed, base_style)));
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

fn inline_spans(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;

    while !rest.is_empty() {
        let Some((pos, token)) = next_token(rest) else {
            spans.push(Span::styled(rest.to_string(), base_style));
            break;
        };

        if pos > 0 {
            spans.push(Span::styled(rest[..pos].to_string(), base_style));
            rest = &rest[pos..];
            continue;
        }

        match token {
            "**" | "__" => {
                if let Some(end) = rest[token.len()..].find(token) {
                    let content = &rest[token.len()..token.len() + end];
                    spans.push(Span::styled(
                        content.to_string(),
                        base_style.add_modifier(Modifier::BOLD),
                    ));
                    rest = &rest[token.len() + end + token.len()..];
                } else {
                    spans.push(Span::styled(token.to_string(), base_style));
                    rest = &rest[token.len()..];
                }
            }
            "`" => {
                if let Some(end) = rest[1..].find('`') {
                    let content = &rest[1..1 + end];
                    spans.push(Span::styled(
                        content.to_string(),
                        base_style.add_modifier(Modifier::BOLD),
                    ));
                    rest = &rest[1 + end + 1..];
                } else {
                    spans.push(Span::styled("`".to_string(), base_style));
                    rest = &rest[1..];
                }
            }
            "*" | "_" => {
                if let Some(end) = rest[token.len()..].find(token) {
                    let content = &rest[token.len()..token.len() + end];
                    spans.push(Span::styled(
                        content.to_string(),
                        base_style.add_modifier(Modifier::ITALIC),
                    ));
                    rest = &rest[token.len() + end + token.len()..];
                } else {
                    spans.push(Span::styled(token.to_string(), base_style));
                    rest = &rest[token.len()..];
                }
            }
            _ => unreachable!("unexpected inline markdown token"),
        }
    }

    spans
}

fn next_token(text: &str) -> Option<(usize, &'static str)> {
    ["**", "__", "`", "*", "_"]
        .into_iter()
        .filter_map(|token| text.find(token).map(|index| (index, token)))
        .min_by_key(|(index, _)| *index)
}
