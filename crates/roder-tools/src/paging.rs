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

pub(crate) fn omitted_lines(page: &LinePage) -> usize {
    page.next_offset
        .map(|next| page.total.saturating_sub(next))
        .unwrap_or(0)
}

pub(crate) fn append_continuation_instruction(
    text: &mut String,
    page: &LinePage,
    continuation_tool: &str,
    continuation_args: &Value,
) {
    if page.next_offset.is_none() {
        return;
    }

    if !text.is_empty() {
        text.push('\n');
    }
    let args_text = serde_json::to_string(continuation_args).unwrap_or_else(|_| "{}".to_string());
    text.push_str(&format!(
        "[truncated: {} more lines omitted. To continue, call {continuation_tool} with {args_text}.]",
        omitted_lines(page)
    ));
}

pub(crate) fn page_metadata(path: String, offset: usize, limit: usize, page: &LinePage) -> Value {
    json!({
        "path": path,
        "offset": offset,
        "limit": limit,
        "shown": page.shown,
        "total_lines": page.total,
        "omitted_lines": omitted_lines(page),
        "next_offset": page.next_offset,
        "truncated": page.next_offset.is_some(),
        "continuation_tool": Value::Null,
        "continuation_args": Value::Null,
    })
}

pub(crate) fn page_metadata_with_continuation(
    path: String,
    offset: usize,
    limit: usize,
    page: &LinePage,
    continuation_tool: &str,
    continuation_args: Value,
) -> Value {
    let mut data = page_metadata(path, offset, limit, page);
    if page.next_offset.is_some() {
        let Some(object) = data.as_object_mut() else {
            return data;
        };
        object.insert("continuation_tool".to_string(), json!(continuation_tool));
        object.insert("continuation_args".to_string(), continuation_args);
    }
    data
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
        assert_eq!(omitted_lines(&page), 2);
        assert!(page.text.contains("line 2\nline 3"));
        assert!(page.text.contains("next_offset=3"));
    }

    #[test]
    fn page_lines_appends_paging_continuation_instruction() {
        let lines = (1..=5)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>();
        let mut text = page_lines(&lines, 0, 2).text;

        append_continuation_instruction(
            &mut text,
            &page_lines(&lines, 0, 2),
            "list_files",
            &json!({"path": ".", "offset": 2, "limit": 2}),
        );

        assert!(text.contains("3 more lines omitted"));
        assert!(text.contains("call list_files"));
        assert!(text.contains("\"offset\":2"));
    }

    #[test]
    fn page_metadata_includes_paging_continuation_args() {
        let lines = (1..=5)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>();
        let page = page_lines(&lines, 0, 2);

        let metadata = page_metadata_with_continuation(
            ".".to_string(),
            0,
            2,
            &page,
            "glob",
            json!({"pattern": "*.rs", "offset": 2, "limit": 2}),
        );

        assert_eq!(metadata["truncated"], true);
        assert_eq!(metadata["omitted_lines"], 3);
        assert_eq!(metadata["continuation_tool"], "glob");
        assert_eq!(metadata["continuation_args"]["offset"], 2);
    }

    #[test]
    fn page_metadata_for_empty_paging_output_has_no_continuation() {
        let page = page_lines(&[], 0, 2);
        let metadata = page_metadata_with_continuation(
            ".".to_string(),
            0,
            2,
            &page,
            "glob",
            json!({"pattern": "*.rs", "offset": 2, "limit": 2}),
        );

        assert_eq!(page.text, "");
        assert_eq!(metadata["truncated"], false);
        assert_eq!(metadata["omitted_lines"], 0);
        assert!(metadata["continuation_tool"].is_null());
        assert!(metadata["continuation_args"].is_null());
    }
}
