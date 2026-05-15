use super::PaletteEntry;

#[derive(Debug, Clone)]
pub struct PaletteMatch<'a> {
    pub entry: &'a PaletteEntry,
    pub score: i32,
}

pub fn search<'a>(
    entries: &'a [PaletteEntry],
    query: &str,
    source_filter: Option<&str>,
) -> Vec<PaletteMatch<'a>> {
    let query = query.trim();
    let mut matches = entries
        .iter()
        .filter(|entry| source_filter.is_none_or(|filter| entry.source_id == filter))
        .filter_map(|entry| {
            let score = score_entry(entry, query)?;
            Some(PaletteMatch { entry, score })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.entry.source_label.cmp(&b.entry.source_label))
            .then_with(|| a.entry.item.title.cmp(&b.entry.item.title))
    });
    matches
}

fn score_entry(entry: &PaletteEntry, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let mut best = score_text(&entry.item.title, query).map(|score| score + 30);
    if let Some(subtitle) = &entry.item.subtitle {
        best = best.max(score_text(subtitle, query).map(|score| score + 10));
    }
    for keyword in &entry.item.keywords {
        best = best.max(score_text(keyword, query));
    }
    best
}

fn score_text(text: &str, query: &str) -> Option<i32> {
    let text = text.to_ascii_lowercase();
    let query = query.to_ascii_lowercase();
    if text == query {
        return Some(300);
    }
    if text.starts_with(&query) {
        return Some(240 - text.len() as i32);
    }
    if let Some(index) = text.find(&query) {
        return Some(180 - index as i32);
    }
    fuzzy_subsequence_score(&text, &query)
}

fn fuzzy_subsequence_score(text: &str, query: &str) -> Option<i32> {
    let mut score = 90;
    let mut last_match: Option<usize> = None;
    let mut chars = text.char_indices();
    for query_char in query.chars() {
        let (index, _) = chars.find(|(_, text_char)| *text_char == query_char)?;
        if let Some(last) = last_match {
            score -= (index - last - 1).min(12) as i32;
        } else {
            score -= index.min(12) as i32;
        }
        last_match = Some(index);
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{PaletteAction, PaletteItem};

    #[test]
    fn exact_and_prefix_matches_rank_before_fuzzy_matches() {
        let entries = vec![
            entry("commands", "Review Code", &["rvw"]),
            entry("commands", "Run Tests", &["test"]),
            entry("commands", "Restart Worker", &[]),
        ];

        let matches = search(&entries, "run", None);
        assert_eq!(
            matches
                .iter()
                .map(|matched| matched.entry.item.title.as_str())
                .collect::<Vec<_>>(),
            ["Run Tests"]
        );

        let matches = search(&entries, "rw", None);
        assert_eq!(matches[0].entry.item.title, "Review Code");
    }

    #[test]
    fn source_filter_limits_results() {
        let entries = vec![
            entry("commands", "Review", &[]),
            entry("models", "Review", &[]),
        ];

        let matches = search(&entries, "review", Some("models"));
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].entry.source_id, "models");
    }

    fn entry(source_id: &str, title: &str, keywords: &[&str]) -> PaletteEntry {
        PaletteEntry {
            source_id: source_id.to_string(),
            source_label: source_id.to_string(),
            item: PaletteItem {
                id: title.to_ascii_lowercase(),
                title: title.to_string(),
                subtitle: None,
                keywords: keywords
                    .iter()
                    .map(|keyword| (*keyword).to_string())
                    .collect(),
                icon: None,
            },
            action: PaletteAction::InsertComposerText(String::new()),
        }
    }
}
