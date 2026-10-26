use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptLink {
    Url { text: String },
    FileReference { path: PathBuf, line: Option<u32> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownLink {
    pub label: String,
    pub destination: String,
    pub consumed: usize,
}

pub fn parse_leading_markdown_link(text: &str) -> Option<MarkdownLink> {
    if text.starts_with('!') || text.starts_with('[') {
        return parse_inline_link(text);
    }
    parse_autolink(text)
}

pub(crate) fn find_link_start(text: &str) -> Option<usize> {
    let mut offset = 0;
    while offset < text.len() {
        let rest = &text[offset..];
        let next = [rest.find('['), rest.find('<')]
            .into_iter()
            .flatten()
            .min()?;
        let abs = offset + next;
        let start = if abs > 0 && text[..abs].ends_with('!') && text[abs..].starts_with('[') {
            abs - '!'.len_utf8()
        } else {
            abs
        };
        if parse_leading_markdown_link(&text[start..]).is_some() {
            return Some(start);
        }
        offset = abs + 1;
    }
    None
}

pub fn linkify_transcript_text(text: &str) -> Vec<TranscriptLink> {
    let mut in_fence = false;
    let mut links = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        links.extend(linkify_line(line));
    }
    links
}

fn linkify_line(line: &str) -> Vec<TranscriptLink> {
    let mut links = Vec::new();
    let mut rest = line;
    let mut in_code = false;
    while !rest.is_empty() {
        if in_code {
            match rest.find('`') {
                Some(end) => {
                    rest = &rest[end + 1..];
                    in_code = false;
                }
                None => break,
            }
            continue;
        }

        let code = rest.find('`');
        let link_at = find_link_start(rest);
        let Some(pos) = earliest(code, link_at) else {
            links.extend(linkify_plain(rest));
            break;
        };
        if pos > 0 {
            links.extend(linkify_plain(&rest[..pos]));
            rest = &rest[pos..];
            continue;
        }
        if rest.starts_with('`') {
            in_code = true;
            rest = &rest[1..];
            continue;
        }
        if let Some(link) = parse_leading_markdown_link(rest) {
            if let Some(transcript) = destination_link(&link.destination) {
                links.push(transcript);
            }
            rest = &rest[link.consumed..];
            continue;
        }
        let ch_len = rest.chars().next().map(|ch| ch.len_utf8()).unwrap_or(1);
        rest = &rest[ch_len..];
    }
    links
}

fn linkify_plain(text: &str) -> Vec<TranscriptLink> {
    let mut links = Vec::new();
    for token in text.split_whitespace() {
        let token = token.trim_matches(|c: char| matches!(c, ',' | ')' | ']' | '"' | '\''));
        if let Some(link) = destination_link(token) {
            links.push(link);
        }
    }
    links
}

fn earliest(left: Option<usize>, right: Option<usize>) -> Option<usize> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(index), None) | (None, Some(index)) => Some(index),
        (None, None) => None,
    }
}

fn destination_link(destination: &str) -> Option<TranscriptLink> {
    let destination = destination.trim();
    if destination.is_empty() {
        return None;
    }
    if is_url(destination) {
        return Some(TranscriptLink::Url {
            text: destination.to_string(),
        });
    }
    file_reference(destination)
}

fn is_url(text: &str) -> bool {
    text.starts_with("https://") || text.starts_with("http://") || text.starts_with("mailto:")
}

fn parse_inline_link(text: &str) -> Option<MarkdownLink> {
    let after_bang = text.strip_prefix('!').unwrap_or(text);
    let rest = after_bang.strip_prefix('[')?;
    let label_end = find_unescaped(rest, ']')?;
    let label = unescape(&rest[..label_end]);
    let after_label = rest.get(label_end + 1..)?.strip_prefix('(')?;
    let (destination, after_dest) = parse_destination(after_label)?;
    let after_title = skip_optional_title(after_dest)?;
    let after_close = after_title.strip_prefix(')')?;
    if destination.is_empty() {
        return None;
    }
    Some(MarkdownLink {
        label,
        destination,
        consumed: text.len() - after_close.len(),
    })
}

fn parse_autolink(text: &str) -> Option<MarkdownLink> {
    let rest = text.strip_prefix('<')?;
    let (scheme, body) = if let Some(body) = rest.strip_prefix("https://") {
        ("https://", body)
    } else if let Some(body) = rest.strip_prefix("http://") {
        ("http://", body)
    } else {
        let body = rest.strip_prefix("mailto:")?;
        ("mailto:", body)
    };
    let end = body.find('>')?;
    let tail = &body[..end];
    if tail.is_empty() || tail.chars().any(char::is_whitespace) {
        return None;
    }
    let destination = format!("{scheme}{tail}");
    let after = body.get(end + 1..)?;
    Some(MarkdownLink {
        label: destination.clone(),
        destination,
        consumed: text.len() - after.len(),
    })
}

fn parse_destination(text: &str) -> Option<(String, &str)> {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix('<') {
        let end = rest.find('>')?;
        let destination = rest[..end].to_string();
        if destination.is_empty() {
            return None;
        }
        return Some((destination, rest.get(end + 1..)?));
    }
    let end = trimmed.find(|ch: char| ch.is_whitespace() || ch == ')')?;
    let destination = trimmed[..end].to_string();
    if destination.is_empty() {
        return None;
    }
    Some((destination, &trimmed[end..]))
}

fn skip_optional_title(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    if trimmed.starts_with(')') {
        return Some(trimmed);
    }
    let (quote, rest) = if let Some(rest) = trimmed.strip_prefix('"') {
        ('"', rest)
    } else {
        let rest = trimmed.strip_prefix('\'')?;
        ('\'', rest)
    };
    let end = rest.find(quote)?;
    Some(rest.get(end + quote.len_utf8()..)?.trim_start())
}

fn find_unescaped(text: &str, needle: char) -> Option<usize> {
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == needle {
            return Some(index);
        }
    }
    None
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut escaped = false;
    for ch in text.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        out.push(ch);
    }
    if escaped {
        out.push('\\');
    }
    out
}

fn file_reference(token: &str) -> Option<TranscriptLink> {
    let (path, line) = match token.rsplit_once(':') {
        Some((path, line)) if !path.is_empty() && line.parse::<u32>().is_ok() => {
            (path, line.parse::<u32>().ok())
        }
        _ => (token, None),
    };
    if !path.contains('/') || path.starts_with("http://") || path.starts_with("https://") {
        return None;
    }
    Some(TranscriptLink::FileReference {
        path: PathBuf::from(path),
        line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linkifier_finds_urls_and_file_refs_outside_code_fences() {
        let links = linkify_transcript_text(
            "See https://example.com and crates/roder-tui/src/app.rs:12\n```text\nhttps://ignored.test\nsrc/ignored.rs:1\n```",
        );

        assert_eq!(
            links,
            vec![
                TranscriptLink::Url {
                    text: "https://example.com".to_string()
                },
                TranscriptLink::FileReference {
                    path: PathBuf::from("crates/roder-tui/src/app.rs"),
                    line: Some(12),
                }
            ]
        );
    }

    #[test]
    fn linkifier_reads_markdown_links_and_skips_inline_code() {
        let links = linkify_transcript_text(
            "See [docs](https://example.com/guide) and [app](crates/roder-tui/src/app.rs:12) plus <https://autolink.test> and `[skip](https://ignored.test)`",
        );

        assert_eq!(
            links,
            vec![
                TranscriptLink::Url {
                    text: "https://example.com/guide".to_string()
                },
                TranscriptLink::FileReference {
                    path: PathBuf::from("crates/roder-tui/src/app.rs"),
                    line: Some(12),
                },
                TranscriptLink::Url {
                    text: "https://autolink.test".to_string()
                },
            ]
        );
    }

    #[test]
    fn markdown_link_keeps_title_and_angle_destination() {
        let titled = parse_leading_markdown_link("[docs](https://example.com/guide \"Guide\")")
            .expect("titled link");
        assert_eq!(titled.label, "docs");
        assert_eq!(titled.destination, "https://example.com/guide");
        assert_eq!(
            titled.consumed,
            "[docs](https://example.com/guide \"Guide\")".len()
        );

        let angled =
            parse_leading_markdown_link("[docs](<https://example.com/a b>)").expect("angled");
        assert_eq!(angled.destination, "https://example.com/a b");
    }
}
