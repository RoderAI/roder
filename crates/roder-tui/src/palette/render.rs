use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem},
};

use super::index::PaletteMatch;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaletteTheme {
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub border: Color,
    pub selection_fg: Color,
    pub selection_bg: Color,
    pub surface_bg: Color,
}

pub fn palette_list<'a>(
    matches: &[PaletteMatch<'a>],
    query: &str,
    source_filter: Option<&str>,
    theme: PaletteTheme,
) -> List<'static> {
    let items = if matches.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No matches",
            Style::default().fg(theme.muted),
        )))]
    } else {
        matches
            .iter()
            .take(12)
            .map(|matched| {
                let icon = matched.entry.item.icon.unwrap_or(' ');
                let mut spans = vec![
                    Span::styled(
                        format!("{icon} "),
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        matched.entry.item.title.clone(),
                        Style::default().fg(theme.text),
                    ),
                ];
                if let Some(subtitle) = &matched.entry.item.subtitle {
                    spans.push(Span::styled(
                        format!("  {}", truncate(subtitle, 64)),
                        Style::default().fg(theme.muted),
                    ));
                }
                spans.push(Span::styled(
                    format!("  [{}]", matched.entry.source_label),
                    Style::default().fg(theme.muted),
                ));
                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    let title = match (query.is_empty(), source_filter) {
        (true, None) => " Command palette ".to_string(),
        (false, None) => format!(" Command palette /{query} "),
        (true, Some(filter)) => format!(" Command palette [{filter}] "),
        (false, Some(filter)) => format!(" Command palette [{filter}] /{query} "),
    };

    List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border))
                .style(Style::default().fg(theme.text).bg(theme.surface_bg))
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .style(Style::default().fg(theme.text).bg(theme.surface_bg))
        .highlight_style(
            Style::default()
                .fg(theme.selection_fg)
                .bg(theme.selection_bg),
        )
        .highlight_symbol("> ")
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if out.len() < value.len() {
        out.push_str("...");
    }
    out
}
