use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FuzzyCandidate {
    pub start_line: usize,
    pub end_line: usize,
    pub score: u32,
    pub snippet: String,
    pub reason: String,
}

pub fn strip_line_number_prefixes(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let digit_count = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
            if digit_count > 0 && trimmed[digit_count..].starts_with(':') {
                trimmed[digit_count + 1..]
                    .strip_prefix(' ')
                    .unwrap_or(&trimmed[digit_count + 1..])
            } else if digit_count > 0 && trimmed[digit_count..].starts_with(" |") {
                trimmed[digit_count + 2..]
                    .strip_prefix(' ')
                    .unwrap_or(&trimmed[digit_count + 2..])
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn normalize_for_match(input: &str) -> String {
    input
        .replace("\r\n", "\n")
        .lines()
        .map(|line| line.trim_end().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("\n")
}

/**
 * Finds the original byte range whose normalized form uniquely matches the
 * normalized needle. Matching is line-wise so byte offsets always refer to
 * the original text — normalization (trailing-whitespace and case folding)
 * must never be used to index into the un-normalized haystack.
 */
pub fn normalized_unique_match_range(
    haystack: &str,
    needle: &str,
) -> Option<std::ops::Range<usize>> {
    let needle_lines: Vec<String> = needle.split('\n').map(normalize_line_for_match).collect();
    if needle_lines.is_empty() || needle_lines.iter().all(|line| line.is_empty()) {
        return None;
    }
    // Byte offset and raw text for every original line.
    let mut line_spans = Vec::new();
    let mut offset = 0;
    for line in haystack.split('\n') {
        line_spans.push((offset, line));
        offset += line.len() + 1;
    }
    let window = needle_lines.len();
    if line_spans.len() < window {
        return None;
    }
    let mut found: Option<std::ops::Range<usize>> = None;
    for start in 0..=(line_spans.len() - window) {
        let matches = (0..window).all(|index| {
            normalize_line_for_match(line_spans[start + index].1) == needle_lines[index]
        });
        if !matches {
            continue;
        }
        if found.is_some() {
            // Two candidate windows match after normalization; refuse.
            return None;
        }
        let (first_offset, _) = line_spans[start];
        let (last_offset, last_line) = line_spans[start + window - 1];
        found = Some(first_offset..last_offset + last_line.len());
    }
    found
}

fn normalize_line_for_match(line: &str) -> String {
    line.trim_end().to_ascii_lowercase()
}

pub fn diagnostic_candidates(haystack: &str, needle: &str, limit: usize) -> Vec<FuzzyCandidate> {
    let needle_lines = needle.lines().count().max(1);
    let normalized_needle = normalize_for_match(needle);
    let lines = haystack.lines().collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for start in 0..lines.len() {
        let end = (start + needle_lines + 1).min(lines.len());
        let snippet = lines[start..end].join("\n");
        let normalized = normalize_for_match(&snippet);
        let score = line_overlap_score(&normalized, &normalized_needle);
        if score > 0 {
            candidates.push(FuzzyCandidate {
                start_line: start + 1,
                end_line: end,
                score,
                snippet,
                reason: "line overlap candidate".to_string(),
            });
        }
    }
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.start_line.cmp(&b.start_line)));
    candidates.truncate(limit);
    candidates
}

fn line_overlap_score(left: &str, right: &str) -> u32 {
    let left = left
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let right = right
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if right.is_empty() {
        return 0;
    }
    let matches = right.iter().filter(|line| left.contains(line)).count();
    ((matches * 100) / right.len()) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colon_and_pipe_line_prefixes() {
        assert_eq!(strip_line_number_prefixes("1: foo\n  2 | bar"), "foo\nbar");
    }

    #[test]
    fn returns_candidates_for_nearby_lines() {
        let candidates = diagnostic_candidates("one\ntwo\nthree", "two\nTHREE", 2);
        assert!(!candidates.is_empty());
        assert!(candidates.iter().any(|candidate| candidate.start_line == 2));
    }
}
