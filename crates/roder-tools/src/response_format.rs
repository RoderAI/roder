use serde::Deserialize;
use serde_json::{Value, json};

const CONCISE_LINE_CHARS: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseFormat {
    Concise,
    Detailed,
}

impl Default for ResponseFormat {
    fn default() -> Self {
        Self::Concise
    }
}

impl ResponseFormat {
    pub(crate) fn schema_property() -> Value {
        json!({
            "type": "string",
            "enum": ["concise", "detailed"],
            "default": "concise",
            "description": "concise keeps model-facing output bounded; detailed preserves full returned line text."
        })
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Concise => "concise",
            Self::Detailed => "detailed",
        }
    }

    pub(crate) fn format_line(self, line: &str) -> String {
        match self {
            Self::Concise => truncate_line(line, CONCISE_LINE_CHARS),
            Self::Detailed => line.to_string(),
        }
    }
}

fn truncate_line(line: &str, max_chars: usize) -> String {
    if line.chars().count() <= max_chars {
        return line.to_string();
    }
    let suffix = "...";
    let keep = max_chars.saturating_sub(suffix.len());
    let mut output = line.chars().take(keep).collect::<String>();
    output.push_str(suffix);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_format_concise_truncates_long_lines() {
        let line = format!("prefix {}", "x".repeat(400));

        let concise = ResponseFormat::Concise.format_line(&line);
        let detailed = ResponseFormat::Detailed.format_line(&line);

        assert!(concise.chars().count() <= CONCISE_LINE_CHARS);
        assert!(concise.ends_with("..."));
        assert_eq!(detailed, line);
    }
}
