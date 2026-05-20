use std::collections::BTreeSet;

use regex::{Regex, RegexBuilder};

use crate::postings::{Trigram, trigrams};
use crate::{SearchError, SearchOptions};

pub(crate) struct CompiledQuery {
    matcher: Regex,
    required_trigrams: BTreeSet<Trigram>,
}

impl CompiledQuery {
    pub(crate) fn compile(options: &SearchOptions) -> Result<Self, SearchError> {
        if options.query.is_empty() {
            return Err(SearchError::InvalidQuery(
                "query must not be empty".to_string(),
            ));
        }

        let pattern = regex_pattern(options);
        let matcher = RegexBuilder::new(&pattern)
            .case_insensitive(!options.case_sensitive)
            .build()
            .map_err(|err| SearchError::InvalidQuery(err.to_string()))?;
        let required_trigrams = required_trigrams(options);

        Ok(Self {
            matcher,
            required_trigrams,
        })
    }

    pub(crate) fn is_match(&self, line: &str) -> bool {
        self.matcher.is_match(line)
    }

    pub(crate) fn required_trigrams(&self) -> &BTreeSet<Trigram> {
        &self.required_trigrams
    }
}

fn regex_pattern(options: &SearchOptions) -> String {
    let body = if options.regex {
        options.query.clone()
    } else {
        regex::escape(&options.query)
    };

    if options.word_boundary {
        format!(r"\b(?:{})\b", body)
    } else {
        body
    }
}

fn required_trigrams(options: &SearchOptions) -> BTreeSet<Trigram> {
    if options.regex {
        regex_literal_runs(&options.query)
            .unwrap_or_default()
            .into_iter()
            .flat_map(|run| trigrams(&run, false))
            .collect()
    } else {
        trigrams(&options.query, false)
    }
}

fn regex_literal_runs(pattern: &str) -> Option<Vec<String>> {
    if pattern
        .chars()
        .any(|ch| matches!(ch, '|' | '[' | '*' | '?' | '{'))
    {
        return None;
    }

    let mut runs = Vec::new();
    let mut current = String::new();
    let mut chars = pattern.chars();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some(escaped) if is_literal_escape(escaped) => current.push(escaped),
                Some(_) => flush_run(&mut current, &mut runs),
                None => current.push('\\'),
            },
            '.' | '^' | '$' | '+' | '(' | ')' => flush_run(&mut current, &mut runs),
            _ => current.push(ch),
        }
    }
    flush_run(&mut current, &mut runs);

    Some(runs)
}

fn is_literal_escape(ch: char) -> bool {
    !matches!(
        ch,
        'd' | 'D' | 's' | 'S' | 'w' | 'W' | 'b' | 'B' | 'A' | 'z' | 'Z'
    )
}

fn flush_run(current: &mut String, runs: &mut Vec<String>) {
    if current.len() >= 3 && current.is_ascii() {
        runs.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SearchMode;

    #[test]
    fn extracts_regex_runs_when_safe() {
        let options = SearchOptions {
            query: "alp.a".to_string(),
            path: ".".into(),
            mode: SearchMode::Auto,
            regex: true,
            case_sensitive: false,
            word_boundary: false,
            max_file_size: crate::DEFAULT_MAX_FILE_SIZE,
        };
        let query = CompiledQuery::compile(&options).unwrap();
        assert!(!query.required_trigrams().is_empty());
    }

    #[test]
    fn skips_ambiguous_regex_runs() {
        assert!(regex_literal_runs("foo|bar").is_none());
        assert!(regex_literal_runs("fo*bar").is_none());
    }
}
