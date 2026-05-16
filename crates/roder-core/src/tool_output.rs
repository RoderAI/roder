const MAX_TOOL_OUTPUT_LINES: usize = 200;

pub(crate) fn cap_tool_output_lines(output: String) -> String {
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
        "[tool output truncated after {MAX_TOOL_OUTPUT_LINES} lines; {remaining} more lines omitted. Use the tool's pagination arguments when available.]"
    ));
    kept.join("\n")
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
    }
}
