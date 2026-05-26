pub fn redact_cursor_secrets(value: &str) -> String {
    value
        .split_whitespace()
        .map(redact_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_token(value: &str) -> String {
    if value.starts_with("crsr_") {
        return "crsr_<redacted>".to_string();
    }
    if value.len() > 48
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return "<redacted-token>".to_string();
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_cursor_key_like_values() {
        let redacted = redact_cursor_secrets("bad crsr_abc123456789 token abc");
        assert!(redacted.contains("crsr_<redacted>"));
        assert!(!redacted.contains("crsr_abc"));
    }
}
