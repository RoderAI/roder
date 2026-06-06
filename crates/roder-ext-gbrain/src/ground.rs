//! Deterministic grounding audit for the concise answerer.
//!
//! After the synthesis call, this proves — with zero LLM cost — which specifics
//! in the draft are *fabricated* (absent from EVERY retrieved record → safe to
//! force-remove) vs *misattributed* (present in the pool but not in the record
//! cited beside them → soft re-check). The flags are handed to the existing
//! delete-only strip call so removal stays grammar-safe.
//!
//! Severity split is the safety contract: only globally-absent spans are
//! "authoritative remove" (a value in no record cannot be faithful, by
//! construction); everything else is a soft hint the LLM adjudicates. Only
//! *specifics* are extracted (names / ISO dates / clock times / verbatim quotes)
//! — never prose — so faithful paraphrase is never touched. Disable wholesale
//! with `GBRAIN_NO_GROUNDING_AUDIT=1`.

use std::collections::{HashMap, HashSet};

use crate::agent::EvidenceItem;

/// One flagged span.
#[derive(Debug, Clone)]
pub struct GFlag {
    pub span: String,
    pub kind: &'static str,
    pub cite: Option<String>,
}

/// Result of the audit, partitioned by how the strip should treat each bucket.
#[derive(Default, Debug)]
pub struct GroundingAudit {
    /// Absent from every record — authoritative remove.
    pub fabricated: Vec<GFlag>,
    /// Present in the pool but not in the cited record — re-check against the cite.
    pub misattributed: Vec<GFlag>,
    /// Walk-through only: slugs from a non-modal event cluster (soft).
    pub off_cluster: Vec<String>,
    /// Affirmative "X was superseded/replaced" sentences whose cited record(s)
    /// contain no replacement language — invented supersession narratives.
    pub unsupported_changes: Vec<String>,
}

impl GroundingAudit {
    pub fn is_empty(&self) -> bool {
        self.fabricated.is_empty()
            && self.misattributed.is_empty()
            && self.off_cluster.is_empty()
            && self.unsupported_changes.is_empty()
    }
}

struct RecIdx {
    text_lc: String,
    dates: HashSet<String>,
}

/// Per-record + pool-wide grounding index.
pub struct GroundIndex {
    by_slug: HashMap<String, RecIdx>,
    pool_lc: String,
    pool_dates: HashSet<String>,
}

pub fn build_ground_index(ev: &[EvidenceItem]) -> GroundIndex {
    let mut by_slug: HashMap<String, RecIdx> = HashMap::new();
    let mut pool_lc = String::new();
    let mut pool_dates: HashSet<String> = HashSet::new();
    for e in ev {
        let mut text_lc = e.text.to_lowercase();
        if !e.source.is_empty() {
            text_lc.push(' ');
            text_lc.push_str(&e.source.to_lowercase());
        }
        let mut dates = HashSet::new();
        collect_dates(&e.text, &mut dates);
        if let Some(iso) = normalize_iso(&e.date) {
            dates.insert(iso); // the structured `.date` field grounds too
        }
        pool_lc.push_str(&text_lc);
        pool_lc.push('\n');
        pool_dates.extend(dates.iter().cloned());
        by_slug
            .entry(e.slug.clone())
            .and_modify(|r| {
                r.text_lc.push('\n');
                r.text_lc.push_str(&text_lc);
                r.dates.extend(dates.iter().cloned());
            })
            .or_insert(RecIdx { text_lc, dates });
    }
    GroundIndex {
        by_slug,
        pool_lc,
        pool_dates,
    }
}

/// Check one extracted span against the pool (fabricated) and the line's cited
/// records (misattributed).
#[allow(clippy::too_many_arguments)]
fn check_span(
    span: String,
    kind: &'static str,
    is_date: bool,
    scope: &[String],
    idx: &GroundIndex,
    fab: &mut Vec<GFlag>,
    mis: &mut Vec<GFlag>,
) {
    let needle = if is_date { span.clone() } else { norm(&span) };
    let in_pool = if is_date {
        idx.pool_dates.contains(&needle)
    } else {
        idx.pool_lc.contains(&needle)
    };
    if !in_pool {
        fab.push(GFlag {
            span,
            kind,
            cite: scope.first().cloned(),
        });
        return;
    }
    if scope.is_empty() {
        return; // present in pool, nothing cited on this line to contradict it
    }
    let in_scope = scope.iter().any(|sl| {
        idx.by_slug.get(sl).is_some_and(|r| {
            if is_date {
                r.dates.contains(&needle)
            } else {
                r.text_lc.contains(&needle)
            }
        })
    });
    if !in_scope {
        mis.push(GFlag {
            span,
            kind,
            cite: scope.first().cloned(),
        });
    }
}

pub fn audit_grounding(
    question: &str,
    answer: &str,
    idx: &GroundIndex,
    walkthrough: bool,
) -> GroundingAudit {
    let real: HashSet<&str> = idx.by_slug.keys().map(String::as_str).collect();
    let ql = question.to_lowercase();
    let mut fab: Vec<GFlag> = Vec::new();
    let mut mis: Vec<GFlag> = Vec::new();
    let mut changes: Vec<String> = Vec::new();
    let mut clusters: Vec<(String, String)> = Vec::new();
    let mut last: Vec<String> = Vec::new();

    for raw in answer.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let here = slugs_in_line(line, &real);
        let scope: Vec<String> = if here.is_empty() { last.clone() } else { here.clone() };
        if !here.is_empty() {
            last = here.clone();
        }
        for s in &here {
            if let Some(c) = event_cluster(s) {
                clusters.push((c.to_string(), s.clone()));
            }
        }
        let heading = is_heading(line);

        // Invented-supersession check: an AFFIRMATIVE change claim whose cited
        // record(s) state no replacement is a fabricated narrative step.
        if !heading {
            let ll = line.to_lowercase();
            if CHANGE_AFFIRM.iter().any(|w| ll.contains(w))
                && !CHANGE_NEG.iter().any(|w| ll.contains(w))
            {
                let supported = scope.iter().any(|sl| {
                    idx.by_slug
                        .get(sl)
                        .is_some_and(|r| SUPPORT_CUE.iter().any(|c| r.text_lc.contains(c)))
                });
                if !supported {
                    let mut t = line.to_string();
                    t.truncate(160);
                    changes.push(t);
                }
            }
        }

        if !heading {
            for d in iso_dates_in(line) {
                check_span(d, "date", true, &scope, idx, &mut fab, &mut mis);
            }
            for t in clock_times_in(line) {
                check_span(t, "time", false, &scope, idx, &mut fab, &mut mis);
            }
            for q in quotes_in(line) {
                check_span(q, "quote", false, &scope, idx, &mut fab, &mut mis);
            }
        }
        for nm in grounded_name_spans(line) {
            if ql.contains(&nm.to_lowercase()) {
                continue; // never flag a party the question itself names
            }
            check_span(nm, "name", false, &scope, idx, &mut fab, &mut mis);
        }
    }

    // Soft cluster-coherence flag, walk-through questions only.
    let mut off = Vec::new();
    if walkthrough && clusters.len() > 1 {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for (c, _) in &clusters {
            *counts.entry(c.as_str()).or_default() += 1;
        }
        if let Some((modal, _)) = counts.iter().max_by_key(|(_, n)| **n) {
            let modal = modal.to_string();
            let mut seen = HashSet::new();
            for (c, s) in &clusters {
                if *c != modal && seen.insert(s.clone()) {
                    off.push(s.clone());
                }
            }
        }
    }
    changes.dedup();
    GroundingAudit {
        fabricated: dedup_flags(fab),
        misattributed: dedup_flags(mis),
        off_cluster: off,
        unsupported_changes: changes,
    }
}

// ---- extractors / helpers ----

/// Event id = slug without its final `-NNN` segment.
pub fn event_cluster(slug: &str) -> Option<&str> {
    slug.rfind('-').map(|i| &slug[..i])
}

fn dedup_flags(v: Vec<GFlag>) -> Vec<GFlag> {
    let mut seen = HashSet::new();
    v.into_iter()
        .filter(|f| seen.insert((f.span.to_lowercase(), f.kind)))
        .collect()
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Bracketed slug-shaped ids on this line that are real retrieved records.
fn slugs_in_line(line: &str, real: &HashSet<&str>) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        if let Some(rel) = rest[open + 1..].find(']') {
            let inner = &rest[open + 1..open + 1 + rel];
            if real.contains(inner) {
                out.push(inner.to_string());
            }
            rest = &rest[open + 1 + rel + 1..];
        } else {
            break;
        }
    }
    out
}

fn is_heading(l: &str) -> bool {
    l.starts_with('#')
        || l.ends_with(':')
        || (l.starts_with("**")
            && l.trim_end().ends_with("**")
            && l.trim_matches('*').split_whitespace().count() <= 5)
}

/// All `YYYY-MM-DD` spans (digit-boundary checked).
fn iso_dates_in(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 10 <= b.len() {
        let d = |k: usize| b[k].is_ascii_digit();
        if d(i) && d(i + 1) && d(i + 2) && d(i + 3) && b[i + 4] == b'-' && d(i + 5) && d(i + 6)
            && b[i + 7] == b'-' && d(i + 8) && d(i + 9)
        {
            let before_ok = i == 0 || !b[i - 1].is_ascii_digit();
            let after_ok = i + 10 >= b.len() || !b[i + 10].is_ascii_digit();
            if before_ok && after_ok {
                out.push(s[i..i + 10].to_string());
                i += 10;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// `H:MM` / `HH:MM` clock times (digit-boundary checked).
fn clock_times_in(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_digit()) {
            let start = i;
            let mut j = i;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let hlen = j - start;
            if (1..=2).contains(&hlen)
                && j + 3 <= b.len()
                && b[j] == b':'
                && b[j + 1].is_ascii_digit()
                && b[j + 2].is_ascii_digit()
                && (j + 3 >= b.len() || !b[j + 3].is_ascii_digit())
            {
                out.push(s[start..j + 3].to_string());
                i = j + 3;
                continue;
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

/// Verbatim-claimed quotes: spans inside `"`/`“”` of >= 5 whitespace tokens.
fn quotes_in(s: &str) -> Vec<String> {
    let norm: String = s.chars().map(|c| if c == '“' || c == '”' { '"' } else { c }).collect();
    let mut out = Vec::new();
    let parts: Vec<&str> = norm.split('"').collect();
    // odd indices are inside quotes
    let mut k = 1;
    while k < parts.len() {
        let inner = parts[k].trim();
        if inner.split_whitespace().count() >= 5 {
            out.push(inner.to_string());
        }
        k += 2;
    }
    out
}

// Affirmative supersession verbs in the ANSWER; negatives that mean "no change"
// (skip them); and the replacement language a real supersession record carries.
const CHANGE_AFFIRM: &[&str] = &[
    "superseded by", "was superseded", "were superseded", "superseding event", "replaced by",
    "was replaced", "were replaced", "later revised", "subsequently revised", "subsequently changed",
    "later changed", "a further change", "was overridden", "was rescinded", "was amended",
    "modified after",
];
const CHANGE_NEG: &[&str] = &[
    "remains current", "remain current", "no record", "not superseded", "no replacement",
    "still in effect", "unchanged", "no later", "never superseded", "no subsequent",
    "no modification", "not modified", "no further", "no superseding", "still stands",
    "no evidence", "does not", "no fact",
];
const SUPPORT_CUE: &[&str] = &[
    "replac", "supersed", "revis", "no longer", "updated to", "changed to", "changed from",
    "instead of", "rather than", "overrid", "rescind", "amend", "in place of", "moving to",
    "moved to", "now ", "raised to", "lowered to", "increased to", "reduced to",
];

const MONTHS: &[&str] = &[
    "january", "february", "march", "april", "may", "june", "july", "august",
    "september", "october", "november", "december",
];

/// Record-side dates: ISO `YYYY-MM-DD` + "Month D, YYYY"/"Month D YYYY" (with a
/// day) normalized to ISO. Month-WITHOUT-day is intentionally NOT parsed.
fn collect_dates(text: &str, out: &mut HashSet<String>) {
    for d in iso_dates_in(text) {
        out.insert(d);
    }
    let lc = text.to_lowercase();
    let toks: Vec<&str> = lc.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).collect();
    for w in toks.windows(3) {
        let (mon, day, year) = (w[0], w[1], w[2]);
        if let Some(mi) = MONTHS.iter().position(|m| *m == mon) {
            let dn: Option<u32> = day.parse().ok();
            let yn: Option<u32> = year.parse().ok();
            if let (Some(dn), Some(yn)) = (dn, yn)
                && (1..=31).contains(&dn)
                && (1900..=2100).contains(&yn)
            {
                out.insert(format!("{yn:04}-{:02}-{:02}", mi + 1, dn));
            }
        }
    }
}

fn normalize_iso(s: &str) -> Option<String> {
    let v = iso_dates_in(s);
    v.into_iter().next()
}

const LABEL_DENY: &[&str] = &[
    "the", "this", "that", "on", "as", "first", "second", "third", "current", "prior",
    "both", "step", "change", "note", "record", "records", "decision", "decisions",
    "position", "positions", "direct", "inferred", "original", "facts", "status", "case",
    "incident", "trigger", "scope", "subsequent", "previous", "next", "final", "summary",
    "conflict", "dispute", "resolution", "rationale", "context", "evidence", "knowledge",
];
const ORG_DENY: &[&str] = &[
    "logistics", "team", "lead", "queue", "board", "engineer", "manager", "coordinator",
    "officer", "director", "makers", "module", "marketplace", "portal", "notes", "report",
    "thread", "email", "slack", "confluence", "jira", "salesforce", "nielsen", "mongodb",
    "postgresql", "api", "sla", "utc", "ui", "sev", "csat", "b2c", "b2b", "sprint",
    "release", "freight", "forwarder", "forwarders", "marketing", "sales", "product",
    "priority", "helix", "transition", "strategy", "initiative", "roadmap", "pilot",
];
const CUES: &[&str] = &[
    "ceo", "coo", "cto", "cfo", "vp", "hr", "lead", "manager", "engineer", "coordinator",
    "officer", "director", "head", "chief", "analyst", "rep", "commander", "attendees",
    "participants", "said", "stated", "announced", "decided", "approved", "proposed",
    "wrote", "confirmed", "replied", "argued", "noted", "joined", "assumed", "assume",
    "took", "agreed", "posted", "emailed", "attended", "presented", "drafted", "flagged",
    "raised", "pushed", "advocated", "objected", "opposed", "made", "identified",
    "suggested", "acknowledged", "sent", "communicated", "named", "appointed", "approval",
    "by",
];

fn is_name_token(w: &str) -> bool {
    let mut chars = w.chars();
    match chars.next() {
        Some(c) if c.is_uppercase() => {}
        _ => return false,
    }
    w.chars().all(|c| c.is_alphabetic()) && w.chars().skip(1).any(|c| c.is_lowercase())
}

fn clean_word(w: &str) -> (&str, bool) {
    let possessive = w.ends_with("'s") || w.ends_with("\u{2019}s");
    let core = w.trim_matches(|c: char| !c.is_alphanumeric());
    (core, possessive)
}

/// Cue-gated person-name bigrams: two `First Last` capitalized tokens (not in the
/// label/org denylists) with a role/verb/possessive/list cue within +-3 tokens.
fn grounded_name_spans(line: &str) -> Vec<String> {
    if is_heading(line) {
        return Vec::new();
    }
    let toks: Vec<&str> = line.split_whitespace().collect();
    let cleaned: Vec<(String, bool)> = toks
        .iter()
        .map(|w| {
            let (c, p) = clean_word(w);
            (c.to_string(), p)
        })
        .collect();
    let mut out = Vec::new();
    let denied = |w: &str| {
        let l = w.to_lowercase();
        LABEL_DENY.contains(&l.as_str()) || ORG_DENY.contains(&l.as_str())
    };
    let mut i = 0usize;
    while i + 1 < cleaned.len() {
        let (a, _) = &cleaned[i];
        let (b, b_poss) = &cleaned[i + 1];
        if a.len() > 1 && b.len() > 1 && is_name_token(a) && is_name_token(b) && !denied(a) && !denied(b)
        {
            // cue search window [i-3, i+4]
            let lo = i.saturating_sub(3);
            let hi = (i + 5).min(cleaned.len());
            let mut cue = *b_poss;
            for k in lo..hi {
                if k == i || k == i + 1 {
                    continue;
                }
                let l = cleaned[k].0.to_lowercase();
                if CUES.contains(&l.as_str()) {
                    cue = true;
                    break;
                }
                // ", Capital" / "and Capital" name-list pattern
                if (l == "and" || toks.get(k).is_some_and(|t| t.starts_with(','))) && k + 1 < cleaned.len()
                    && is_name_token(&cleaned[k + 1].0)
                {
                    cue = true;
                    break;
                }
            }
            if cue {
                out.push(format!("{a} {b}"));
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Walk-through / justification questions (enumerate evidence) — excludes the
/// as-of / what-changed audit phrasings so C4 multi-cluster answers aren't flagged.
pub fn is_walkthrough_question(question: &str) -> bool {
    let q = question.to_lowercase();
    if q.contains("as of") || q.contains("changed") || q.contains("since") {
        return false;
    }
    ["walk me through", "walk through", "justif", "supporting records", "supporting evidence",
     "which records", "which documents", "evidence", "step by step", "for each"]
        .iter()
        .any(|m| q.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(slug: &str, date: &str, text: &str) -> EvidenceItem {
        EvidenceItem {
            index: 1,
            slug: slug.into(),
            date: date.into(),
            source: String::new(),
            status: "current".into(),
            note: String::new(),
            text: text.into(),
        }
    }

    #[test]
    fn names_flagged_only_with_a_cue_and_not_orgs_or_headings() {
        assert_eq!(grounded_name_spans("the decision was made by Omar Khalil"), vec!["Omar Khalil"]);
        assert_eq!(grounded_name_spans("Luis Gomez approved the change"), vec!["Luis Gomez"]);
        // orgs / role-phrases / headings must NOT be flagged
        assert!(grounded_name_spans("Helix Logistics shifted strategy").is_empty());
        assert!(grounded_name_spans("the Priority Queue tool launched").is_empty());
        assert!(grounded_name_spans("the Decision Makers met").is_empty());
        assert!(grounded_name_spans("## Sales Lead Summary").is_empty());
    }

    #[test]
    fn audit_separates_fabricated_misattributed_and_ignores_month_only() {
        let pool = vec![
            ev("ART-EV-2023-015-002", "2023-02-20", "On 2023-02-20 the SLA target rose to 99.9%."),
            ev("ART-EV-2023-030-001", "2023-04-05", "Aisha Patel drafted the plan in May 2023."),
        ];
        let idx = build_ground_index(&pool);
        // date present in record 015 but cited to 030 => misattributed
        let a1 = audit_grounding("q", "The change was on 2023-02-20 [ART-EV-2023-030-001].", &idx, false);
        assert!(a1.misattributed.iter().any(|f| f.span == "2023-02-20"));
        // date in NO record => fabricated
        let a2 = audit_grounding("q", "A later change on 2023-08-15 [ART-EV-2023-015-002].", &idx, false);
        assert!(a2.fabricated.iter().any(|f| f.span == "2023-08-15"));
        // month-only "May 2023" is never extracted/flagged
        let a3 = audit_grounding("q", "Aisha Patel acted in May 2023 [ART-EV-2023-030-001].", &idx, false);
        assert!(a3.fabricated.is_empty() && a3.misattributed.is_empty());
    }

    #[test]
    fn structured_date_field_grounds_and_clusters_split() {
        let pool = vec![ev("ART-EV-2023-015-002", "2023-02-20", "SLA target rose to 99.9%.")];
        let idx = build_ground_index(&pool);
        // 2023-02-20 only in the .date field, correctly cited => not flagged
        let a = audit_grounding("q", "Raised on 2023-02-20 [ART-EV-2023-015-002].", &idx, false);
        assert!(a.fabricated.is_empty() && a.misattributed.is_empty());
        assert_eq!(event_cluster("ART-EV-2023-015-002"), Some("ART-EV-2023-015"));
    }

    #[test]
    fn unsupported_supersession_flagged_but_real_and_negative_kept() {
        let pool = vec![
            ev("ART-EV-2023-004-001", "2023-02-10", "Nia Osei is leaving Helix to join a competitor."),
            ev("ART-EV-2023-015-002", "2023-03-05", "The SLA target was raised to 99.9%, replacing the 99.5% goal."),
        ];
        let idx = build_ground_index(&pool);
        // invented supersession: cited record has no replacement language -> flagged
        let a = audit_grounding(
            "q",
            "Her exit was superseded by a 2023-09-15 reorg [ART-EV-2023-004-001].",
            &idx,
            false,
        );
        assert!(!a.unsupported_changes.is_empty());
        // real supersession: cited record literally states the replacement -> NOT flagged
        let b = audit_grounding(
            "q",
            "The 99.5% goal was superseded by 99.9% [ART-EV-2023-015-002].",
            &idx,
            false,
        );
        assert!(b.unsupported_changes.is_empty());
        // negative "remains current" claim -> NOT flagged
        let c = audit_grounding(
            "q",
            "No record supersedes this fact; it remains current [ART-EV-2023-004-001].",
            &idx,
            false,
        );
        assert!(c.unsupported_changes.is_empty());
    }

    #[test]
    fn empty_audit_when_clean() {
        let pool = vec![ev("R-1", "2023-01-01", "Maya Patel decided to launch on 2023-01-01.")];
        let idx = build_ground_index(&pool);
        let a = audit_grounding("who decided?", "Maya Patel decided it [R-1].", &idx, false);
        assert!(a.is_empty());
    }
}
