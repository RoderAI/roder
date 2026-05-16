use serde_json::{Value, json};

pub(crate) const DEFAULT_PAGE_LINES: usize = 200;
pub(crate) const MAX_PAGE_LINES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinePage {
    pub(crate) text: String,
    pub(crate) shown: usize,
    pub(crate) total: usize,
    pub(crate) next_offset: Option<usize>,
}

pub(crate) fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_PAGE_LINES).clamp(1, MAX_PAGE_LINES)
}

pub(crate) fn page_lines(lines: &[String], offset: usize, limit: usize) -> LinePage {
    let total = lines.len();
    let offset = offset.min(total);
    let end = offset.saturating_add(limit).min(total);
    let shown = end.saturating_sub(offset);
    let next_offset = (end < total).then_some(end);
    let mut text = lines[offset..end].join("\n");
    if let Some(next) = next_offset {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!(
            "[showing lines {}-{} of {total}; next_offset={next}]",
            offset + 1,
            end
        ));
    }
    LinePage {
        text,
        shown,
        total,
        next_offset,
    }
}

pub(crate) fn page_metadata(path: String, offset: usize, limit: usize, page: &LinePage) -> Value {
    json!({
        "path": path,
        "offset": offset,
        "limit": limit,
        "shown": page.shown,
        "total_lines": page.total,
        "next_offset": page.next_offset,
        "truncated": page.next_offset.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_lines_reports_next_offset() {
        let lines = (1..=5)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>();
        let page = page_lines(&lines, 1, 2);

        assert_eq!(page.shown, 2);
        assert_eq!(page.next_offset, Some(3));
        assert!(page.text.contains("line 2\nline 3"));
        assert!(page.text.contains("next_offset=3"));
    }
}
