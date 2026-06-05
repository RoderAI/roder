#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadFormatOptions {
    pub start_line: usize,
    pub limit: usize,
}

impl Default for ReadFormatOptions {
    fn default() -> Self {
        Self {
            start_line: 1,
            limit: 200,
        }
    }
}

pub fn format_line_numbered_read(text: &str, options: ReadFormatOptions) -> String {
    let start_line = options.start_line.max(1);
    let limit = options.limit.max(1);
    text.lines()
        .enumerate()
        .skip(start_line - 1)
        .take(limit)
        .map(|(index, line)| format!("{:>5}: {}", index + 1, line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_plain_line_numbers() {
        let output = format_line_numbered_read(
            "a\nb\nc",
            ReadFormatOptions {
                start_line: 2,
                limit: 1,
            },
        );
        assert_eq!(output, "    2: b");
    }
}
