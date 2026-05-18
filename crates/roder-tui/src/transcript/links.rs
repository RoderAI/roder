use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptLink {
    Url { text: String },
    FileReference { path: PathBuf, line: Option<u32> },
}

pub fn linkify_transcript_text(text: &str) -> Vec<TranscriptLink> {
    let mut in_fence = false;
    let mut links = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for token in line.split_whitespace() {
            let token = token.trim_matches(|c: char| matches!(c, ',' | ')' | ']' | '"' | '\''));
            if token.starts_with("http://") || token.starts_with("https://") {
                links.push(TranscriptLink::Url {
                    text: token.to_string(),
                });
                continue;
            }
            if let Some(link) = file_reference(token) {
                links.push(link);
            }
        }
    }
    links
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
}
