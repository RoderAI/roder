const MAX_TOOL_OUTPUT_LINES: usize = 200;
const MAX_TOOL_OUTPUT_CHARS: usize = 20_000;
const ARTIFACT_INLINE_CHARS: usize = 6_000;
const NO_CONTINUATION_NOTE: &str = "No continuation cursor is available for this capped aggregate output; rerun the original tool with narrower pagination or filters when available.";

pub(crate) fn cap_tool_output_lines(output: String) -> String {
    let original_line_count = output.lines().count();
    let line_capped = cap_tool_output_line_count(output);
    cap_tool_output_chars(line_capped, original_line_count)
}

pub(crate) fn should_spill_tool_output(output: &str) -> bool {
    output.lines().count() > MAX_TOOL_OUTPUT_LINES || output.chars().count() > MAX_TOOL_OUTPUT_CHARS
}

pub(crate) fn artifact_backed_tool_output(
    output: &str,
    artifact_reference: &str,
    label: &str,
) -> String {
    let line_count = output.lines().count();
    let byte_count = output.len();
    let snippet = truncate_middle_chars(output, ARTIFACT_INLINE_CHARS);
    format!(
        "Tool output was stored in a local context artifact because it exceeded inline limits.\n\
         Label: {label}\n\
         Total output lines: {line_count}\n\
         Total output bytes: {byte_count}\n\n\
         {artifact_reference}\n\n\
         Inline excerpt:\n{snippet}"
    )
}

fn cap_tool_output_line_count(output: String) -> String {
    let mut lines = output.lines();
    let mut kept = Vec::new();
    for _ in 0..MAX_TOOL_OUTPUT_LINES {
        let Some(line) = lines.next() else {
            return output;
        };
        kept.push(line.to_string());
    }

    let remaining = lines.count();
    if remaining == 0 {
        return output;
    }

    kept.push(format!(
        "[tool output truncated after {MAX_TOOL_OUTPUT_LINES} lines; {remaining} more lines omitted. {NO_CONTINUATION_NOTE}]"
    ));
    kept.join("\n")
}

fn cap_tool_output_chars(output: String, original_line_count: usize) -> String {
    if output.chars().count() <= MAX_TOOL_OUTPUT_CHARS {
        return output;
    }

    let prefix = format!("Total output lines: {original_line_count}\n\n");
    let content_budget = MAX_TOOL_OUTPUT_CHARS.saturating_sub(prefix.chars().count());
    format!("{prefix}{}", truncate_middle_chars(&output, content_budget))
}

fn truncate_middle_chars(text: &str, max_chars: usize) -> String {
    let total_chars = text.chars().count();
    if total_chars <= max_chars {
        return text.to_string();
    }

    let marker = format!(
        "\n[tool output truncated; {} chars omitted. {NO_CONTINUATION_NOTE}]\n",
        total_chars.saturating_sub(max_chars)
    );
    let marker_chars = marker.chars().count();
    if marker_chars >= max_chars {
        return marker.chars().take(max_chars).collect();
    }

    let remaining = max_chars - marker_chars;
    let head_chars = remaining / 2;
    let tail_chars = remaining - head_chars;
    let head = text.chars().take(head_chars).collect::<String>();
    let tail = text
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();

    format!("{head}{marker}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_tool_output_lines_with_notice() {
        let output = (1..=205)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        let capped = cap_tool_output_lines(output);

        assert_eq!(capped.lines().count(), 201);
        assert!(capped.contains("line 200"));
        assert!(capped.contains("5 more lines omitted"));
        assert!(capped.contains("No continuation cursor is available"));
    }

    #[test]
    fn caps_single_huge_line_by_char_budget_with_notice() {
        let output = format!("start{}end", "x".repeat(MAX_TOOL_OUTPUT_CHARS * 2));

        let capped = cap_tool_output_lines(output);

        assert!(capped.chars().count() <= MAX_TOOL_OUTPUT_CHARS);
        assert!(capped.starts_with("Total output lines: 1\n\nstart"));
        assert!(capped.ends_with("end"));
        assert!(capped.contains("chars omitted"));
        assert!(capped.contains("No continuation cursor is available"));
    }

    #[test]
    fn caps_multi_line_output_by_char_budget_even_under_line_limit() {
        let output = (1..=50)
            .map(|line| format!("line {line}: {}", "x".repeat(1_000)))
            .collect::<Vec<_>>()
            .join("\n");

        let capped = cap_tool_output_lines(output);

        assert!(capped.chars().count() <= MAX_TOOL_OUTPUT_CHARS);
        assert!(capped.starts_with("Total output lines: 50\n\nline 1:"));
        assert!(capped.contains("chars omitted"));
        assert!(capped.contains("No continuation cursor is available"));
    }

    #[test]
    fn leaves_empty_tool_output_unchanged() {
        let capped = cap_tool_output_lines(String::new());

        assert_eq!(capped, "");
    }

    #[test]
    fn artifact_backed_summary_stays_bounded_and_names_reference() {
        let output = format!("start{}end", "x".repeat(MAX_TOOL_OUTPUT_CHARS * 2));
        let summary = artifact_backed_tool_output(
            &output,
            "[artifact: tool_output stdout lines=1 bytes=40008 id=artifact-1]",
            "stdout",
        );

        assert!(should_spill_tool_output(&output));
        assert!(summary.chars().count() < MAX_TOOL_OUTPUT_CHARS);
        assert!(summary.contains("artifact-1"));
        assert!(summary.contains("Inline excerpt"));
        assert!(summary.ends_with("end"));
    }
}
