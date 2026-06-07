use serde::{Deserialize, Serialize};

use super::ConfidenceLabel;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTextChunk {
    pub source_fact_id: String,
    pub artifact_slug: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatedQuoteSpan {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamStatement {
    pub id: String,
    pub source_fact_id: String,
    pub artifact_slug: String,
    pub text: String,
    pub quote: ValidatedQuoteSpan,
    pub temporal_cues: Vec<String>,
    pub confidence: ConfidenceLabel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractionError {
    EmptyQuote,
    InvalidSpan {
        start: usize,
        end: usize,
        len: usize,
    },
    QuoteMismatch {
        expected: String,
        actual: String,
    },
}

impl std::fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyQuote => write!(f, "quote span is empty"),
            Self::InvalidSpan { start, end, len } => {
                write!(f, "quote span {start}..{end} is outside text length {len}")
            }
            Self::QuoteMismatch { expected, actual } => {
                write!(
                    f,
                    "quote span text mismatch: expected {expected:?}, got {actual:?}"
                )
            }
        }
    }
}

impl std::error::Error for ExtractionError {}

pub trait StatementExtractor {
    fn extract(&self, chunk: &RawTextChunk) -> Result<StatementExtraction, ExtractionError>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatementExtraction {
    pub statements: Vec<DreamStatement>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicStatementExtractor;

impl StatementExtractor for DeterministicStatementExtractor {
    fn extract(&self, chunk: &RawTextChunk) -> Result<StatementExtraction, ExtractionError> {
        Ok(StatementExtraction {
            statements: extract_statements(chunk)?,
        })
    }
}

pub fn extract_statements(chunk: &RawTextChunk) -> Result<Vec<DreamStatement>, ExtractionError> {
    let mut statements = Vec::new();
    let mut start = None;

    for (idx, ch) in chunk.text.char_indices() {
        if start.is_none() && !ch.is_whitespace() {
            start = Some(idx);
        }

        if is_sentence_boundary(&chunk.text, idx, ch)
            && let Some(statement_start) = start.take()
        {
            push_statement(chunk, statement_start, idx + ch.len_utf8(), &mut statements)?;
        }
    }

    if let Some(statement_start) = start {
        push_statement(chunk, statement_start, chunk.text.len(), &mut statements)?;
    }

    Ok(statements)
}

pub fn validate_quote_span(
    raw_text: &str,
    start: usize,
    end: usize,
    expected_quote: &str,
) -> Result<ValidatedQuoteSpan, ExtractionError> {
    if expected_quote.trim().is_empty() {
        return Err(ExtractionError::EmptyQuote);
    }
    if start >= end || end > raw_text.len() {
        return Err(ExtractionError::InvalidSpan {
            start,
            end,
            len: raw_text.len(),
        });
    }
    if !raw_text.is_char_boundary(start) || !raw_text.is_char_boundary(end) {
        return Err(ExtractionError::InvalidSpan {
            start,
            end,
            len: raw_text.len(),
        });
    }

    let actual = &raw_text[start..end];
    if actual == expected_quote
        || normalize_for_quote(actual) == normalize_for_quote(expected_quote)
    {
        return Ok(ValidatedQuoteSpan {
            start,
            end,
            text: actual.to_string(),
        });
    }

    Err(ExtractionError::QuoteMismatch {
        expected: expected_quote.to_string(),
        actual: actual.to_string(),
    })
}

fn push_statement(
    chunk: &RawTextChunk,
    start: usize,
    end: usize,
    statements: &mut Vec<DreamStatement>,
) -> Result<(), ExtractionError> {
    let quote = chunk.text[start..end].trim();
    if quote.len() < 3 {
        return Ok(());
    }
    let leading = chunk.text[start..end].find(quote).unwrap_or(0);
    let quote_start = start + leading;
    let quote_end = quote_start + quote.len();
    let quote_span = validate_quote_span(&chunk.text, quote_start, quote_end, quote)?;
    let ordinal = statements.len() + 1;
    statements.push(DreamStatement {
        id: format!("statement:{}:{ordinal:04}", chunk.source_fact_id),
        source_fact_id: chunk.source_fact_id.clone(),
        artifact_slug: chunk.artifact_slug.clone(),
        text: quote.trim_end_matches(['.', '!', '?']).trim().to_string(),
        quote: quote_span,
        temporal_cues: temporal_cues(quote),
        confidence: ConfidenceLabel::Extracted,
    });
    Ok(())
}

fn temporal_cues(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    [
        "today",
        "yesterday",
        "tomorrow",
        "now",
        "currently",
        "previously",
        "effective",
        "as of",
        "replaces",
        "supersedes",
    ]
    .into_iter()
    .filter(|cue| lower.contains(cue))
    .map(str::to_string)
    .collect()
}

fn is_sentence_boundary(raw_text: &str, idx: usize, ch: char) -> bool {
    if ch == '\n' || ch == '!' || ch == '?' {
        return true;
    }
    if ch != '.' {
        return false;
    }

    let previous = raw_text[..idx].chars().next_back();
    let next = raw_text[idx + ch.len_utf8()..].chars().next();
    !matches!((previous, next), (Some(left), Some(right)) if left.is_ascii_digit() && right.is_ascii_digit())
}

fn normalize_for_quote(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}
