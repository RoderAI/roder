//! gbrain-style hybrid retrieval: fuse dense-vector similarity, a BM25-lite
//! lexical signal, and a graph boost from the supersession/contradiction links
//! so related facts in a chain surface together. Pure functions over candidates
//! the store has already restricted to an `AsOf` snapshot.

use std::collections::{HashMap, HashSet};

use crate::model::TemporalFact;

const ALPHA_VECTOR: f32 = 0.6;
const BETA_LEXICAL: f32 = 0.4;
const GAMMA_GRAPH: f32 = 0.15;
const GRAPH_TOP: usize = 5;

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "of", "is", "was", "were", "what", "who", "did", "does", "and", "or", "to",
    "in", "on", "for", "by", "as", "at", "be", "it", "that", "this", "with", "from", "are",
];

/// A fact and its (optional) stored vector under the active embedding backend.
pub struct Candidate {
    pub fact: TemporalFact,
    pub vector: Option<Vec<f32>>,
}

/// A fact with its fused relevance score and component breakdown.
pub struct Scored {
    pub fact: TemporalFact,
    pub score: f32,
    pub vector_score: f32,
    pub lexical_score: f32,
}

pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .collect()
}

/// Fraction of distinct query terms present in `text` (BM25-lite).
pub fn lexical_score(terms: &[String], text: &str) -> f32 {
    if terms.is_empty() {
        return 0.0;
    }
    let lower = text.to_lowercase();
    let distinct: HashSet<&String> = terms.iter().collect();
    let found = distinct
        .iter()
        .filter(|t| lower.contains(t.as_str()))
        .count();
    found as f32 / distinct.len() as f32
}

/// Fuse the three signals and return candidates sorted by descending score.
///
/// `neighbors` maps a fact id to the ids it is linked to (supersedes /
/// contradicts, both directions); a candidate linked to a high-ranked fact gets
/// a bounded boost so chains stay together.
pub fn fuse(
    query: &str,
    query_vector: &[f32],
    candidates: Vec<Candidate>,
    neighbors: &HashMap<String, Vec<String>>,
) -> Vec<Scored> {
    let terms = tokenize(query);

    // Pass 1: base score = α·vector + β·lexical.
    struct Base {
        fact: TemporalFact,
        vector_score: f32,
        lexical_score: f32,
        base: f32,
    }
    let mut bases: Vec<Base> = candidates
        .into_iter()
        .map(|c| {
            let vector_score = c
                .vector
                .as_deref()
                .map(|v| crate::embed::cosine(query_vector, v).clamp(0.0, 1.0))
                .unwrap_or(0.0);
            let lexical_score = lexical_score(&terms, &c.fact.text);
            let base = ALPHA_VECTOR * vector_score + BETA_LEXICAL * lexical_score;
            Base {
                fact: c.fact,
                vector_score,
                lexical_score,
                base,
            }
        })
        .collect();

    // Identify the top-ranked ids by base score for the graph boost.
    let mut ranked: Vec<(usize, f32)> =
        bases.iter().enumerate().map(|(i, b)| (i, b.base)).collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top_ids: HashSet<String> = ranked
        .iter()
        .take(GRAPH_TOP)
        .filter(|(i, score)| *score > 0.0 && !bases[*i].fact.id.is_empty())
        .map(|(i, _)| bases[*i].fact.id.clone())
        .collect();

    // Pass 2: add the graph boost for candidates adjacent to a top hit.
    let mut scored: Vec<Scored> = bases
        .drain(..)
        .map(|b| {
            let boost = neighbors
                .get(&b.fact.id)
                .map(|links| {
                    if links.iter().any(|id| top_ids.contains(id)) {
                        GAMMA_GRAPH
                    } else {
                        0.0
                    }
                })
                .unwrap_or(0.0);
            Scored {
                score: b.base + boost,
                vector_score: b.vector_score,
                lexical_score: b.lexical_score,
                fact: b.fact,
            }
        })
        .collect();

    // Deterministic ordering: score desc, then more-recent valid_at, then id.
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.fact.valid_at.cmp(&a.fact.valid_at))
            .then(a.fact.id.cmp(&b.fact.id))
    });
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::content_hash;
    use roder_api::memory::MemoryScope;
    use time::OffsetDateTime;

    fn fact(id: &str, text: &str) -> TemporalFact {
        let now = OffsetDateTime::UNIX_EPOCH;
        TemporalFact {
            id: id.into(),
            scope: MemoryScope::Global,
            subject: None,
            text: text.into(),
            metadata: serde_json::Value::Null,
            valid_at: now,
            invalid_at: None,
            ingested_at: now,
            expired_at: None,
            supersedes: None,
            superseded_by: None,
            supersession_reason: None,
            provenance: vec![],
            content_hash: content_hash(text),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn lexical_relevance_orders_results() {
        let q = "acme account owner";
        let qv = crate::embed::local_embedding(q);
        let candidates = vec![
            Candidate {
                vector: Some(crate::embed::local_embedding(
                    "the acme account owner is maya",
                )),
                fact: fact("a", "the acme account owner is maya"),
            },
            Candidate {
                vector: Some(crate::embed::local_embedding(
                    "unrelated kubernetes scaling note",
                )),
                fact: fact("b", "unrelated kubernetes scaling note"),
            },
        ];
        let out = fuse(q, &qv, candidates, &HashMap::new());
        assert_eq!(out[0].fact.id, "a");
        assert!(out[0].score > out[1].score);
    }

    #[test]
    fn graph_boost_surfaces_linked_fact() {
        let q = "acme owner";
        let qv = crate::embed::local_embedding(q);
        // "b" is irrelevant on its own but linked to top hit "a".
        let candidates = vec![
            Candidate {
                vector: Some(crate::embed::local_embedding("acme owner is maya patel")),
                fact: fact("a", "acme owner is maya patel"),
            },
            Candidate {
                vector: Some(crate::embed::local_embedding("zzz unrelated noise text")),
                fact: fact("b", "zzz unrelated noise text"),
            },
            Candidate {
                vector: Some(crate::embed::local_embedding("yyy other unrelated noise")),
                fact: fact("c", "yyy other unrelated noise"),
            },
        ];
        let mut neighbors = HashMap::new();
        neighbors.insert("b".to_string(), vec!["a".to_string()]);
        let out = fuse(q, &qv, candidates, &neighbors);
        let pos_b = out.iter().position(|s| s.fact.id == "b").unwrap();
        let pos_c = out.iter().position(|s| s.fact.id == "c").unwrap();
        assert!(pos_b < pos_c, "linked fact b should outrank unlinked c");
    }
}
