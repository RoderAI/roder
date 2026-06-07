//! Bi-temporal data model for the gbrain memory store.
//!
//! The model follows the OrgMemBench paper (design.tex §3.2): every fact carries
//! four timestamps spanning two timelines —
//! * **valid time** (the world): [`valid_at`](TemporalFact::valid_at) ..
//!   [`invalid_at`](TemporalFact::invalid_at)
//! * **transaction time** (the record): [`ingested_at`](TemporalFact::ingested_at)
//!   .. [`expired_at`](TemporalFact::expired_at)
//!
//! Facts are *invalidated, never deleted*, so the prior state of any fact stays
//! recoverable. An [`AsOf`] snapshot reconstructs what the organization believed
//! on a past date.

use roder_api::memory::MemoryScope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub use crate::dream::{
    ConfidenceLabel, DreamMode, DreamPolicy, DreamRun, DreamStatus, EvidenceCard, GraphEdge,
    GraphHyperedge, GraphNode, TemporalEvent, normalize_graph_id, validate_graph_edge_endpoints,
};

/// One bi-temporal fact in the store. Rows are never hard-deleted; a retraction
/// sets [`expired_at`](Self::expired_at) and a real-world change sets
/// [`invalid_at`](Self::invalid_at) plus a supersession link.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TemporalFact {
    pub id: String,
    pub scope: MemoryScope,
    /// The entity/key a fact is about — the grouping key for supersession and
    /// contradiction detection (e.g. "acme-owner", or a thread id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub text: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// When the fact became true in the world.
    #[serde(with = "time::serde::rfc3339")]
    pub valid_at: OffsetDateTime,
    /// When the fact stopped being true in the world (None = still valid).
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub invalid_at: Option<OffsetDateTime>,
    /// When the organization recorded the fact (transaction-time start).
    #[serde(with = "time::serde::rfc3339")]
    pub ingested_at: OffsetDateTime,
    /// When the record was retracted/corrected (transaction-time end).
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expired_at: Option<OffsetDateTime>,
    /// The fact this one replaces (set on the *new* fact).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// The fact that replaced this one (set on the *old* fact).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// Why the supersession happened (explainable, not merely detectable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersession_reason: Option<String>,
    /// Source artifact ids / slugs backing this fact (provenance).
    #[serde(default)]
    pub provenance: Vec<String>,
    pub content_hash: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl TemporalFact {
    /// Lifecycle status of the fact relative to *now* — used for audit-replay
    /// annotations (C4: "flag what has since changed").
    pub fn status(&self, now: OffsetDateTime) -> FactStatus {
        if self.expired_at.is_some_and(|e| e <= now) {
            FactStatus::Retracted
        } else if self.superseded_by.is_some() {
            FactStatus::Superseded
        } else if self.invalid_at.is_some_and(|i| i <= now) {
            FactStatus::Invalidated
        } else {
            FactStatus::Current
        }
    }

    /// True when the record exists in transaction time at `tt`
    /// (ingested by `tt` and not yet retracted).
    pub fn transaction_visible(&self, tt: OffsetDateTime) -> bool {
        self.ingested_at <= tt && self.expired_at.is_none_or(|e| e > tt)
    }

    /// True when the fact was valid in the world at `vt`.
    pub fn valid_in_world(&self, vt: OffsetDateTime) -> bool {
        self.valid_at <= vt && self.invalid_at.is_none_or(|i| i > vt)
    }
}

/// Lifecycle status of a fact relative to a reference time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactStatus {
    Current,
    Superseded,
    Invalidated,
    Retracted,
}

impl FactStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            FactStatus::Current => "current",
            FactStatus::Superseded => "superseded",
            FactStatus::Invalidated => "invalidated",
            FactStatus::Retracted => "retracted",
        }
    }
}

/// A bi-temporal query point. `None` on either axis means "current" (now).
///
/// * `transaction_time` — what records the org *had* (and had not retracted).
/// * `valid_time` — what was *true in the world*.
///
/// "What did the org believe as of date D?" sets both to D.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AsOf {
    pub transaction_time: Option<OffsetDateTime>,
    pub valid_time: Option<OffsetDateTime>,
}

impl AsOf {
    /// Current belief (now on both axes).
    pub fn now() -> Self {
        Self::default()
    }

    /// "As of date D" — both axes pinned to the same instant.
    pub fn at(instant: OffsetDateTime) -> Self {
        Self {
            transaction_time: Some(instant),
            valid_time: Some(instant),
        }
    }

    /// True when `fact` is visible in this snapshot, resolving open axes to `now`.
    pub fn visible(&self, fact: &TemporalFact, now: OffsetDateTime) -> bool {
        let tt = self.transaction_time.unwrap_or(now);
        let vt = self.valid_time.unwrap_or(now);
        fact.transaction_visible(tt) && fact.valid_in_world(vt)
    }

    /// The instant the snapshot is anchored at (for status annotations); falls
    /// back to `now` when the snapshot is "current".
    pub fn anchor(&self, now: OffsetDateTime) -> OffsetDateTime {
        self.valid_time.or(self.transaction_time).unwrap_or(now)
    }

    pub fn is_current(&self) -> bool {
        self.transaction_time.is_none() && self.valid_time.is_none()
    }
}

/// SHA-256 hex digest of text (stable content hash for dedup).
pub fn content_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Format an instant as RFC3339 for SQLite storage.
pub fn format_time(time: OffsetDateTime) -> String {
    time.format(&Rfc3339)
        .unwrap_or_else(|_| OffsetDateTime::UNIX_EPOCH.to_string())
}

/// Parse an RFC3339 instant.
pub fn parse_time(input: &str) -> anyhow::Result<OffsetDateTime> {
    Ok(OffsetDateTime::parse(input, &Rfc3339)?)
}

/// Parse a flexible date: full RFC3339, or a bare `YYYY-MM-DD` (interpreted as
/// midnight UTC). Used for `valid_at`/`as_of` inputs that arrive as ISO dates.
pub fn parse_flexible(input: &str) -> anyhow::Result<OffsetDateTime> {
    let trimmed = input.trim();
    if let Ok(dt) = parse_time(trimmed) {
        return Ok(dt);
    }
    // Bare date: YYYY-MM-DD -> midnight UTC.
    let parts: Vec<&str> = trimmed.split(['-', '/']).collect();
    if parts.len() == 3 {
        let year: i32 = parts[0].parse()?;
        let month: u8 = parts[1].parse()?;
        let day: u8 = parts[2].parse()?;
        let month = time::Month::try_from(month)
            .map_err(|_| anyhow::anyhow!("invalid month {month} in date {trimmed:?}"))?;
        let date = time::Date::from_calendar_date(year, month, day)?;
        return Ok(time::PrimitiveDateTime::new(date, time::Time::MIDNIGHT).assume_utc());
    }
    anyhow::bail!("could not parse date/time: {input:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> OffsetDateTime {
        parse_flexible(s).unwrap()
    }

    fn fact(valid: &str, invalid: Option<&str>, expired: Option<&str>) -> TemporalFact {
        let now = dt("2026-01-01");
        TemporalFact {
            id: "f".into(),
            scope: MemoryScope::Global,
            subject: Some("s".into()),
            text: "t".into(),
            metadata: serde_json::Value::Null,
            valid_at: dt(valid),
            invalid_at: invalid.map(dt),
            ingested_at: dt(valid),
            expired_at: expired.map(dt),
            supersedes: None,
            superseded_by: None,
            supersession_reason: None,
            provenance: vec![],
            content_hash: content_hash("t"),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn parse_flexible_handles_date_and_rfc3339() {
        assert!(parse_flexible("2022-01-15").is_ok());
        assert!(parse_flexible("2022-01-15T10:00:00Z").is_ok());
        assert!(parse_flexible("not-a-date").is_err());
    }

    #[test]
    fn as_of_valid_time_only() {
        // Valid 2022..2024.
        let f = fact("2022-01-01", Some("2024-01-01"), None);
        let now = dt("2026-01-01");
        // Valid-time 2023 -> visible; 2025 -> not (invalidated).
        assert!(AsOf::at(dt("2023-01-01")).visible(&f, now));
        assert!(!AsOf::at(dt("2025-01-01")).visible(&f, now));
    }

    #[test]
    fn as_of_transaction_time_only() {
        // Ingested 2022, retracted 2024.
        let mut f = fact("2022-01-01", None, Some("2024-01-01"));
        f.ingested_at = dt("2022-01-01");
        let now = dt("2026-01-01");
        // Transaction-time 2023 still sees the (later retracted) record.
        assert!(AsOf::at(dt("2023-01-01")).visible(&f, now));
        // Transaction-time 2025 (after retraction) does not.
        assert!(!AsOf::at(dt("2025-01-01")).visible(&f, now));
    }

    #[test]
    fn current_belief_excludes_invalidated_and_retracted() {
        let now = dt("2026-01-01");
        let invalidated = fact("2020-01-01", Some("2023-01-01"), None);
        let retracted = fact("2020-01-01", None, Some("2023-01-01"));
        let live = fact("2020-01-01", None, None);
        assert!(!AsOf::now().visible(&invalidated, now));
        assert!(!AsOf::now().visible(&retracted, now));
        assert!(AsOf::now().visible(&live, now));
    }

    #[test]
    fn status_reflects_lifecycle() {
        let now = dt("2026-01-01");
        let mut superseded = fact("2020-01-01", Some("2023-01-01"), None);
        superseded.superseded_by = Some("g".into());
        assert_eq!(superseded.status(now), FactStatus::Superseded);
        assert_eq!(
            fact("2020-01-01", None, Some("2023-01-01")).status(now),
            FactStatus::Retracted
        );
        assert_eq!(
            fact("2020-01-01", Some("2023-01-01"), None).status(now),
            FactStatus::Invalidated
        );
        assert_eq!(
            fact("2020-01-01", None, None).status(now),
            FactStatus::Current
        );
    }
}
