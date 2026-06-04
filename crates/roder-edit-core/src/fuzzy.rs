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

pub fn normalized_unique_match(haystack: &str, needle: &str) -> Option<usize> {
    let normalized_haystack = normalize_for_match(haystack);
    let normalized_needle = normalize_for_match(needle);
    let first = normalized_haystack.find(&normalized_needle)?;
    if normalized_haystack[first + normalized_needle.len()..]
        .find(&normalized_needle)
        .is_some()
    {
        return None;
    }
    Some(first)
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
