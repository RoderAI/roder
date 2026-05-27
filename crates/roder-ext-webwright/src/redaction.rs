const REDACTED_LINE: &str = "[redacted sensitive Webwright output line]";

pub(crate) fn redact_sensitive_text(text: &str) -> String {
    text.lines()
        .map(redact_sensitive_line)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn redact_sensitive_line(line: &str) -> String {
    if is_sensitive_line(line) {
        REDACTED_LINE.to_string()
    } else {
        line.to_string()
    }
}

fn is_sensitive_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("authorization:")
        || lower.contains("bearer ")
        || lower.contains("api_key")
        || lower.contains("api-key")
        || lower.contains("api key")
        || lower.contains("password=")
        || lower.contains("password:")
        || lower.contains("secret=")
        || lower.contains("secret:")
        || lower.contains("token=")
        || lower.contains("token:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_common_secret_lines() {
        assert_eq!(
            redact_sensitive_line("Authorization: Bearer abc123"),
            REDACTED_LINE
        );
        assert_eq!(
            redact_sensitive_line("final datum: Fixture Heading"),
            "final datum: Fixture Heading"
        );
    }
}
