use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    pub start: usize,
    pub end: usize,
    pub target: LinkTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    Url(String),
    File { path: PathBuf, line: Option<u32> },
}

pub fn link_spans(text: &str) -> Vec<LinkSpan> {
    let mut spans = Vec::new();
    let mut offset = 0usize;
    let mut in_fence = false;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches('\n');
        if content.trim_start().starts_with("```") {
            in_fence = !in_fence;
            offset += line.len();
            continue;
        }
        if !in_fence {
            spans.extend(line_link_spans(content, offset));
        }
        offset += line.len();
    }
    spans
}

fn line_link_spans(line: &str, line_offset: usize) -> Vec<LinkSpan> {
    let mut spans = Vec::new();
    let mut search = 0usize;
    for token in line.split_whitespace() {
        let Some(relative_start) = line[search..].find(token) else {
            continue;
        };
        let raw_start = search + relative_start;
        search = raw_start + token.len();
        let (trimmed, leading, trailing) = trim_token(token);
        if trimmed.is_empty() {
            continue;
        }
        let start = line_offset + raw_start + leading;
        let end = line_offset + raw_start + token.len() - trailing;
        if let Some(url) = url_target(trimmed) {
            spans.push(LinkSpan {
                start,
                end,
                target: LinkTarget::Url(url),
            });
            continue;
        }
        if let Some((path, line)) = file_target(trimmed) {
            spans.push(LinkSpan {
                start,
                end,
                target: LinkTarget::File { path, line },
            });
        }
    }
    spans
}

fn trim_token(token: &str) -> (&str, usize, usize) {
    let leading = token
        .bytes()
        .take_while(|byte| matches!(byte, b'(' | b'[' | b'{' | b'<' | b'"' | b'\''))
        .count();
    let trailing = token
        .bytes()
        .rev()
        .take_while(|byte| matches!(byte, b'.' | b',' | b')' | b']' | b'}' | b'>' | b'"' | b'\''))
        .count();
    if leading + trailing >= token.len() {
        return ("", leading, trailing);
    }
    (&token[leading..token.len() - trailing], leading, trailing)
}

fn url_target(token: &str) -> Option<String> {
    (token.starts_with("https://") || token.starts_with("http://")).then(|| token.to_string())
}

fn file_target(token: &str) -> Option<(PathBuf, Option<u32>)> {
    if token.starts_with("http://") || token.starts_with("https://") {
        return None;
    }
    let (path, line) = split_line_suffix(token);
    if !looks_like_path(path) {
        return None;
    }
    Some((PathBuf::from(path), line))
}

fn split_line_suffix(token: &str) -> (&str, Option<u32>) {
    let Some((path, line)) = token.rsplit_once(':') else {
        return (token, None);
    };
    if line.is_empty() || !line.bytes().all(|byte| byte.is_ascii_digit()) {
        return (token, None);
    }
    (path, line.parse().ok())
}

fn looks_like_path(path: &str) -> bool {
    if path.len() < 3 || !path.contains('/') {
        return false;
    }
    path.starts_with("./")
        || path.starts_with("../")
        || path.starts_with('/')
        || path.ends_with(".rs")
        || path.ends_with(".toml")
        || path.ends_with(".md")
        || path.ends_with(".json")
        || path.ends_with(".ts")
        || path.ends_with(".tsx")
        || path.ends_with(".js")
        || path.ends_with(".jsx")
        || path.ends_with(".go")
        || path.ends_with(".py")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linkifier_detects_urls_and_file_references() {
        let spans = link_spans("See https://example.com and crates/roder-tui/src/app.rs:42.");

        assert_eq!(spans.len(), 2);
        assert_eq!(
            spans[0].target,
            LinkTarget::Url("https://example.com".to_string())
        );
        assert_eq!(
            spans[1].target,
            LinkTarget::File {
                path: PathBuf::from("crates/roder-tui/src/app.rs"),
                line: Some(42)
            }
        );
    }

    #[test]
    fn linkifier_ignores_code_fences() {
        let spans = link_spans("```rust\nsee https://example.com and crates/a.rs:1\n```\n");

        assert!(spans.is_empty());
    }
}
