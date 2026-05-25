pub(super) fn trim_output_buffer_to_max_bytes(
    output: &mut String,
    cursor: &mut usize,
    max_bytes: usize,
) {
    if output.len() <= max_bytes {
        return;
    }

    let keep_from = output.len() - max_bytes;
    let remove_to = next_char_boundary(output, keep_from);
    output.replace_range(..remove_to, "");
    *cursor = cursor.saturating_sub(remove_to).min(output.len());
}

pub(super) fn truncate_output(
    output: &str,
    max_output_tokens: Option<usize>,
    default_max_output_tokens: usize,
) -> String {
    let max_bytes = max_output_tokens
        .unwrap_or(default_max_output_tokens)
        .saturating_mul(4);
    if output.len() <= max_bytes {
        return output.trim_end().to_string();
    }

    let keep_from = output.len() - max_bytes;
    let suffix_start = next_char_boundary(output, keep_from);
    let suffix = output[suffix_start..].trim_end();
    format!("[{} bytes omitted]\n{suffix}", suffix_start)
}

pub(super) fn format_exec_output(
    exit_code: Option<i32>,
    status: &str,
    duration_ms: u64,
    session_id: Option<u64>,
    output: &str,
) -> String {
    let exit = exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "still running".to_string());
    let mut text = format!(
        "Exit code: {exit}\nStatus: {status}\nWall time: {:.3} seconds",
        duration_ms as f64 / 1000.0
    );
    if status == "running"
        && let Some(session_id) = session_id
    {
        text.push_str(&format!("\nSession id: {session_id}"));
    }
    text.push_str(&format!("\nOutput:\n{output}"));
    text
}

fn next_char_boundary(text: &str, start: usize) -> usize {
    if start >= text.len() {
        return text.len();
    }

    let mut index = start;
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_output_on_utf8_boundary() {
        let output = format!("{}{}", "a".repeat(20), "é".repeat(20));

        let truncated = truncate_output(&output, Some(5), 6000);

        assert!(truncated.starts_with("["));
        assert!(truncated.contains("bytes omitted]\n"));
        assert!(truncated.ends_with('é'));
    }

    #[test]
    fn trims_output_buffer_on_utf8_boundary_and_updates_cursor() {
        let mut output = format!("{}{}", "é".repeat(8), "tail");
        let mut cursor = output.len();

        trim_output_buffer_to_max_bytes(&mut output, &mut cursor, 5);

        assert_eq!(output, "tail");
        assert_eq!(cursor, output.len());
    }

    #[test]
    fn trimming_buffer_resets_cursor_when_unread_prefix_is_removed() {
        let mut output = format!("{}{}", "é".repeat(8), "tail");
        let mut cursor = 2;

        trim_output_buffer_to_max_bytes(&mut output, &mut cursor, 5);

        assert_eq!(output, "tail");
        assert_eq!(cursor, 0);
    }
}
