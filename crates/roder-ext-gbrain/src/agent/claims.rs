//! Deterministic claim-ledger validation for strict faithfulness mode.
//!
//! The LLM may propose claims, but strict mode only admits claims whose cited
//! record ids exist, whose quote spans are present in those records, and whose
//! obvious specifics are copyable from the cited evidence.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimType {
    Direct,
    Derived,
    TemporalStatus,
    Contradiction,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimTemporalScope {
    Current,
    AsOf,
    SinceAsOf,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimConfidence {
    Proven,
    Inferred,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteSpan {
    pub artifact_id: String,
    pub record_number: usize,
    pub quote: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimSupport {
    pub artifact_id: String,
    pub record_number: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerClaim {
    pub claim_id: String,
    pub claim_text: String,
    pub claim_type: ClaimType,
    #[serde(default)]
    pub supporting_artifact_ids: Vec<String>,
    #[serde(default)]
    pub supporting_record_numbers: Vec<usize>,
    #[serde(default)]
    pub quote_spans: Vec<QuoteSpan>,
    pub temporal_scope: ClaimTemporalScope,
    pub confidence: ClaimConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimLedger {
    pub claims: Vec<LedgerClaim>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub record_number: usize,
    pub artifact_id: String,
    pub date: String,
    pub status: String,
    pub note: String,
    pub text: String,
}

impl EvidenceRecord {
    pub fn new(
        record_number: usize,
        artifact_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            record_number,
            artifact_id: artifact_id.into(),
            date: String::new(),
            status: String::new(),
            note: String::new(),
            text: text.into(),
        }
    }

    pub fn with_date(mut self, date: impl Into<String>) -> Self {
        self.date = date.into();
        self
    }

    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = status.into();
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    fn searchable_text(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.artifact_id, self.date, self.status, self.note, self.text
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerFailure {
    pub claim_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimVerdict {
    pub claim_id: String,
    pub confidence: ClaimConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaithfulnessTrace {
    pub verified: Vec<LedgerClaim>,
    pub rejected: Vec<LedgerClaim>,
    pub failures: Vec<LedgerFailure>,
}

impl FaithfulnessTrace {
    pub fn is_fully_verified(&self) -> bool {
        self.rejected.is_empty() && self.failures.is_empty()
    }
}

pub fn validate_claim_ledger(
    claims: &[LedgerClaim],
    evidence: &[EvidenceRecord],
) -> FaithfulnessTrace {
    let by_record: HashMap<usize, &EvidenceRecord> = evidence
        .iter()
        .map(|record| (record.record_number, record))
        .collect();
    let by_artifact: HashMap<&str, &EvidenceRecord> = evidence
        .iter()
        .map(|record| (record.artifact_id.as_str(), record))
        .collect();

    let mut trace = FaithfulnessTrace::default();
    for claim in claims {
        let mut checked = claim.clone();
        let mut reasons = validate_one(&checked, &by_record, &by_artifact);
        if reasons.is_empty() {
            checked.confidence = match checked.claim_type {
                ClaimType::Derived | ClaimType::TemporalStatus | ClaimType::Contradiction => {
                    ClaimConfidence::Inferred
                }
                ClaimType::Direct => ClaimConfidence::Proven,
                ClaimType::Unsupported => ClaimConfidence::Rejected,
            };
            checked.rejection_reason = None;
            trace.verified.push(checked);
        } else {
            reasons.sort();
            reasons.dedup();
            checked.confidence = ClaimConfidence::Rejected;
            checked.rejection_reason = Some(reasons.join("; "));
            for reason in &reasons {
                trace.failures.push(LedgerFailure {
                    claim_id: checked.claim_id.clone(),
                    reason: reason.clone(),
                });
            }
            trace.rejected.push(checked);
        }
    }
    trace
}

fn validate_one(
    claim: &LedgerClaim,
    by_record: &HashMap<usize, &EvidenceRecord>,
    by_artifact: &HashMap<&str, &EvidenceRecord>,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if claim.claim_type == ClaimType::Unsupported {
        reasons.push("claim is explicitly marked unsupported".to_string());
    }
    if claim.supporting_record_numbers.is_empty() {
        reasons.push("claim has no supporting record numbers".to_string());
    }
    if claim.supporting_artifact_ids.is_empty() {
        reasons.push("claim has no supporting artifact ids".to_string());
    }
    if claim.quote_spans.is_empty() {
        reasons.push("claim has no quote spans".to_string());
    }
    if claim.claim_type == ClaimType::Direct
        && (dedup_count(&claim.supporting_record_numbers) > 1
            || dedup_count(&claim.supporting_artifact_ids) > 1)
    {
        reasons.push(
            "cross-record claim must be typed as derived, temporal_status, or contradiction"
                .to_string(),
        );
    }

    for record_number in &claim.supporting_record_numbers {
        if !by_record.contains_key(record_number) {
            reasons.push(format!("supporting record {record_number} does not exist"));
        }
    }
    for artifact_id in &claim.supporting_artifact_ids {
        if !by_artifact.contains_key(artifact_id.as_str()) {
            reasons.push(format!("supporting artifact {artifact_id} does not exist"));
        }
    }

    let supported_records: Vec<&EvidenceRecord> = claim
        .supporting_record_numbers
        .iter()
        .filter_map(|record_number| by_record.get(record_number).copied())
        .collect();
    let supported_artifacts: HashSet<&str> = claim
        .supporting_artifact_ids
        .iter()
        .map(String::as_str)
        .collect();
    for record in &supported_records {
        if !supported_artifacts.contains(record.artifact_id.as_str()) {
            reasons.push(format!(
                "record {} is not paired with artifact {} in claim support",
                record.record_number, record.artifact_id
            ));
        }
    }

    for span in &claim.quote_spans {
        match by_record.get(&span.record_number) {
            Some(record) if record.artifact_id != span.artifact_id => reasons.push(format!(
                "quote span record {} belongs to {}, not {}",
                span.record_number, record.artifact_id, span.artifact_id
            )),
            Some(record) => {
                if span.quote.trim().is_empty() {
                    reasons.push(format!("quote span for {} is empty", span.artifact_id));
                } else if !normalized_contains(&record.text, &span.quote) {
                    reasons.push(format!(
                        "quote span for {} is not present in cited record {}",
                        span.artifact_id, span.record_number
                    ));
                }
            }
            None => reasons.push(format!(
                "quote span cites missing record {}",
                span.record_number
            )),
        }
    }

    if !supported_records.is_empty() {
        let support_text = supported_records
            .iter()
            .map(|record| record.searchable_text())
            .collect::<Vec<_>>()
            .join("\n");
        for specific in extract_specifics(&claim.claim_text) {
            if !normalized_contains(&support_text, &specific) {
                reasons.push(format!(
                    "specific not supported by cited evidence: {specific}"
                ));
            }
        }
        if claim.temporal_scope == ClaimTemporalScope::SinceAsOf
            && !has_explicit_change_language(&support_text)
            && !claim_text_says_unchanged(&claim.claim_text)
        {
            reasons.push(
                "since_as_of claim lacks explicit change, replacement, or unchanged support"
                    .to_string(),
            );
        }
    }

    reasons
}

pub fn extract_specifics(text: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    out.extend(extract_artifact_ids(text));
    out.extend(extract_iso_dates(text));
    out.extend(extract_times(text));
    out.extend(extract_numbers(text));
    out.extend(extract_named_spans(text));
    out.into_iter().collect()
}

fn extract_artifact_ids(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .filter(|token| {
            token.len() >= 6
                && token.contains('-')
                && token.chars().any(|c| c.is_ascii_digit())
                && token.chars().any(|c| c.is_ascii_uppercase())
        })
        .map(str::to_string)
        .collect()
}

fn extract_iso_dates(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for i in 0..bytes.len().saturating_sub(9) {
        let span = &bytes[i..i + 10];
        if span[0].is_ascii_digit()
            && span[1].is_ascii_digit()
            && span[2].is_ascii_digit()
            && span[3].is_ascii_digit()
            && span[4] == b'-'
            && span[5].is_ascii_digit()
            && span[6].is_ascii_digit()
            && span[7] == b'-'
            && span[8].is_ascii_digit()
            && span[9].is_ascii_digit()
        {
            out.push(text[i..i + 10].to_string());
        }
    }
    out
}

fn extract_times(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(clean_word)
        .filter(|token| {
            let bytes = token.as_bytes();
            bytes.len() == 5
                && bytes[0].is_ascii_digit()
                && bytes[1].is_ascii_digit()
                && bytes[2] == b':'
                && bytes[3].is_ascii_digit()
                && bytes[4].is_ascii_digit()
        })
        .collect()
}

fn extract_numbers(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split_whitespace() {
        let token = clean_word(raw);
        if token.is_empty() || token.contains('-') {
            continue;
        }
        let has_digit = token.chars().any(|c| c.is_ascii_digit());
        let numeric = token
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '$' | '%' | ',' | '.' | ':' | 'x' | 'X'));
        if has_digit && numeric {
            out.push(token);
        }
    }
    out
}

fn extract_named_spans(text: &str) -> Vec<String> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(clean_word)
        .filter(|word| !word.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        if !is_named_token(&words[i]) {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < words.len() && is_named_token(&words[i]) && i - start < 4 {
            i += 1;
        }
        let span = words[start..i].join(" ");
        if i - start >= 2 || is_single_named_token(&span) {
            out.push(span);
        }
    }
    out
}

fn clean_word(raw: &str) -> String {
    raw.trim_matches(|c: char| {
        !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.' | '%' | '$'))
    })
    .trim_end_matches("'s")
    .trim_end_matches("'")
    .to_string()
}

fn is_named_token(word: &str) -> bool {
    if is_stopword(word) {
        return false;
    }
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first.is_ascii_uppercase() && chars.clone().any(|c| c.is_ascii_lowercase()) {
        return true;
    }
    word.len() >= 2
        && word
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn is_single_named_token(word: &str) -> bool {
    word.len() >= 4 || word.chars().all(|c| c.is_ascii_uppercase())
}

fn is_stopword(word: &str) -> bool {
    matches!(
        word,
        "A" | "An"
            | "And"
            | "As"
            | "At"
            | "Because"
            | "But"
            | "By"
            | "Current"
            | "Direct"
            | "Evidence"
            | "For"
            | "From"
            | "If"
            | "In"
            | "No"
            | "Not"
            | "Of"
            | "On"
            | "Or"
            | "Record"
            | "Since"
            | "That"
            | "The"
            | "Then"
            | "This"
            | "To"
            | "Unknown"
            | "With"
            | "Without"
    )
}

fn normalized_contains(haystack: &str, needle: &str) -> bool {
    let needle = normalize(needle);
    !needle.is_empty() && normalize(haystack).contains(&needle)
}

fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = true;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn has_explicit_change_language(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "changed",
        "change",
        "replaced",
        "replacement",
        "superseded",
        "supersedes",
        "revised",
        "updated",
        "amended",
        "still current",
        "unchanged",
        "still in effect",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn claim_text_says_unchanged(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("unchanged")
        || lower.contains("still current")
        || lower.contains("still in effect")
}

fn dedup_count<T>(items: &[T]) -> usize
where
    T: Eq + std::hash::Hash,
{
    items.iter().collect::<HashSet<_>>().len()
}
