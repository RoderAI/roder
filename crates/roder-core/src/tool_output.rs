use roder_api::artifacts::{
    ContextArtifact, ContextArtifactKind, ContextArtifactReference, format_artifact_reference,
};

use crate::artifacts::ContextArtifactStore;

const MAX_TOOL_OUTPUT_LINES: usize = 200;
const MAX_TOOL_OUTPUT_CHARS: usize = 20_000;
const INLINE_TAIL_LINES: usize = 20;

pub(crate) struct ToolOutputCapResult {
    pub text: String,
    pub artifact: Option<ContextArtifact>,
}

pub(crate) fn cap_tool_output_lines(output: String) -> String {
    cap_tool_output_lines_with_store(output, None, None, None, None)
        .text
}

pub(crate) fn cap_tool_output_lines_with_store(
    output: String,
    store: Option<&ContextArtifactStore>,
    thread_id: Option<&str>,
    turn_id: Option<&str>,
    tool_id: Option<&str>,
) -> ToolOutputCapResult {
    if !needs_artifact(&output) {
        return ToolOutputCapResult {
            text: cap_tool_output_lines_legacy(output),
            artifact: None,
        };
    }

    let Some(store) = store else {
        return ToolOutputCapResult {
            text: cap_tool_output_lines_legacy(output),
            artifact: None,
        };
    };
    let (thread_id, turn_id, tool_id) = match (thread_id, turn_id, tool_id) {
        (Some(thread_id), Some(turn_id), Some(tool_id)) => (thread_id, turn_id, tool_id),
        _ => {
            return ToolOutputCapResult {
                text: cap_tool_output_lines_legacy(output),
                artifact: None,
            };
        }
    };

    let artifact = store
        .write(
            thread_id,
            turn_id,
            ContextArtifactKind::ToolOutput,
            tool_id,
            Some(tool_id),
            "stdout",
            output.as_bytes(),
        )
        .ok();
    let reference = artifact.as_ref().map(|artifact| {
        ContextArtifactReference::from_artifact(artifact, "stdout")
    });
    let tail = tail_lines(&output, INLINE_TAIL_LINES);
    let mut text = String::new();
    if let Some(reference) = reference.as_ref() {
        text.push_str(&format_artifact_reference(reference));
        text.push_str("\n\n");
    }
    text.push_str("Tail snippet:\n");
    text.push_str(&tail);
    ToolOutputCapResult { text, artifact }
}

fn needs_artifact(output: &str) -> bool {
    output.lines().count() > MAX_TOOL_OUTPUT_LINES || output.chars().count() > MAX_TOOL_OUTPUT_CHARS
}

fn tail_lines(output: &str, lines: usize) -> String {
    let all: Vec<&str> = output.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

fn cap_tool_output_lines_legacy(output: String) -> String {
    let original_line_count = output.lines().count();
    let line_capped = cap_tool_output_line_count(output);
    cap_tool_output_chars(line_capped, original_line_count)
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
        "[tool output truncated after {MAX_TOOL_OUTPUT_LINES} lines; {remaining} more lines omitted. Use read_artifact or grep_artifact to inspect more.]"
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
        "\n[tool output truncated; {} chars omitted. Use read_artifact or grep_artifact to inspect more.]\n",
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
    use crate::artifacts::ContextArtifactStore;

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
    }

    #[test]
    fn caps_single_huge_line_by_char_budget_with_notice() {
        let output = format!("start{}end", "x".repeat(MAX_TOOL_OUTPUT_CHARS * 2));

        let capped = cap_tool_output_lines(output);

        assert!(capped.chars().count() <= MAX_TOOL_OUTPUT_CHARS);
        assert!(capped.starts_with("Total output lines: 1\n\nstart"));
        assert!(capped.ends_with("end"));
        assert!(capped.contains("chars omitted"));
        assert!(capped.contains("read_artifact"));
    }

    #[test]
    fn writes_huge_output_to_artifact_with_reference() {
        let store = ContextArtifactStore::new(
            std::env::temp_dir().join(format!("roder-tool-output-{}", uuid::Uuid::new_v4())),
        );
        let output = (1..=300)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let capped = cap_tool_output_lines_with_store(
            output,
            Some(&store),
            Some("thread"),
            Some("turn"),
            Some("call_1"),
        );
        assert!(capped.artifact.is_some());
        assert!(capped.text.contains("[artifact: tool_output"));
        assert!(capped.text.contains("line 300"));
    }

    #[test]
    fn binary_safe_output_stores_lossy_utf8() {
        let store = ContextArtifactStore::new(
            std::env::temp_dir().join(format!("roder-tool-output-bin-{}", uuid::Uuid::new_v4())),
        );
        let output = String::from_utf8_lossy(&[0xff, b'a', b'\n', b'b']).into_owned();
        let capped = cap_tool_output_lines_with_store(
            output.repeat(500),
            Some(&store),
            Some("thread"),
            Some("turn"),
            Some("call_bin"),
        );
        assert!(capped.artifact.is_some());
    }
}
