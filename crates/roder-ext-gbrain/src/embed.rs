//! Embedding backend for hybrid recall.
//!
//! Uses a real [`EmbeddingProvider`] (e.g. OpenAI) when one is supplied;
//! otherwise falls back to a deterministic local hash embedding so the crate
//! builds, tests, and runs fully offline and reproducibly. Each produced
//! [`Embedding`] is self-describing (`provider_id` + `model`) so stored vectors
//! are only ever compared against query vectors from the same backend.

use std::sync::Arc;

use roder_api::embeddings::{EmbeddingProvider, EmbeddingRequest};
use sha2::{Digest, Sha256};

pub const LOCAL_PROVIDER: &str = "local";
pub const LOCAL_MODEL: &str = "hash-256";
pub const LOCAL_DIMENSIONS: usize = 256;

/// Provider embed attempts before degrading to the local fallback.
const EMBED_ATTEMPTS: usize = 2;

/// A vector plus the backend identity that produced it.
#[derive(Debug, Clone)]
pub struct Embedding {
    pub provider_id: String,
    pub model: String,
    pub values: Vec<f32>,
}

/// Chooses the embedding backend once, at construction.
#[derive(Clone)]
pub struct Embedder {
    provider: Option<Arc<dyn EmbeddingProvider>>,
    provider_id: String,
    model: String,
}

impl Embedder {
    pub fn new(provider: Option<Arc<dyn EmbeddingProvider>>) -> Self {
        match &provider {
            Some(p) => {
                let descriptor = p.descriptor();
                Self {
                    provider_id: descriptor.id,
                    model: descriptor.default_model,
                    provider: provider.clone(),
                }
            }
            None => Self {
                provider: None,
                provider_id: LOCAL_PROVIDER.to_string(),
                model: LOCAL_MODEL.to_string(),
            },
        }
    }

    /// Backend identity this embedder *prefers* (a provider call may still fall
    /// back to local on error, in which case the returned [`Embedding`] reports
    /// the local identity).
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn embed(&self, text: &str) -> Embedding {
        if let Some(provider) = &self.provider {
            let request = EmbeddingRequest {
                model: self.model.clone(),
                inputs: vec![text.to_string()],
                dimensions: None,
            };
            // Retry transient provider errors before degrading. A silent fallback
            // here relabels the query as `local`, which makes its vectors match
            // none of the provider-tagged stored vectors at recall time — so the
            // fallback is both retried and made observable.
            let mut last_err: Option<String> = None;
            for _ in 0..EMBED_ATTEMPTS {
                match provider.embed(request.clone()).await {
                    Ok(response) => {
                        if let Some(vector) = response.embeddings.into_iter().next()
                            && !vector.values.is_empty() {
                                return Embedding {
                                    provider_id: self.provider_id.clone(),
                                    model: self.model.clone(),
                                    values: vector.values,
                                };
                            }
                        last_err = Some("provider returned no embedding".to_string());
                        break;
                    }
                    Err(err) => last_err = Some(err.to_string()),
                }
            }
            if let Some(err) = last_err {
                eprintln!(
                    "gbrain: embedding provider '{}' unavailable ({err}); using deterministic \
                     local fallback for this call (vector scoring degraded)",
                    self.provider_id
                );
            }
        }
        Embedding {
            provider_id: LOCAL_PROVIDER.to_string(),
            model: LOCAL_MODEL.to_string(),
            values: local_embedding(text),
        }
    }
}

/// Deterministic local embedding: hashes tokens and adjacent bigrams into a
/// fixed-width bag-of-features vector, then L2-normalizes. Cheap, offline, and
/// stable across runs — good enough for tests and as a graceful fallback.
pub fn local_embedding(text: &str) -> Vec<f32> {
    let mut values = vec![0.0_f32; LOCAL_DIMENSIONS];
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();
    for token in &tokens {
        values[bucket(token)] += 1.0;
    }
    // Adjacent bigrams add a little word-order signal.
    for pair in tokens.windows(2) {
        let bigram = format!("{} {}", pair[0], pair[1]);
        values[bucket(&bigram)] += 0.5;
    }
    normalize(values)
}

fn bucket(token: &str) -> usize {
    let digest = Sha256::digest(token.as_bytes());
    let idx = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) as usize;
    idx % LOCAL_DIMENSIONS
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut a_norm = 0.0;
    let mut b_norm = 0.0;
    for (left, right) in a.iter().zip(b.iter()) {
        dot += left * right;
        a_norm += left * left;
        b_norm += right * right;
    }
    if a_norm == 0.0 || b_norm == 0.0 {
        0.0
    } else {
        dot / (a_norm.sqrt() * b_norm.sqrt())
    }
}

pub fn encode(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

pub fn decode(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn normalize(mut values: Vec<f32>) -> Vec<f32> {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut values {
            *value /= norm;
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_embedding_is_deterministic_and_discriminative() {
        let q = local_embedding("who owns the acme account");
        let related = local_embedding("the acme account owner is maya");
        let unrelated = local_embedding("kubernetes cluster autoscaling policy");
        assert_eq!(q, local_embedding("who owns the acme account"));
        assert!(cosine(&q, &related) > cosine(&q, &unrelated));
    }

    #[test]
    fn encode_decode_roundtrips() {
        let v = local_embedding("roundtrip vector");
        assert_eq!(decode(&encode(&v)), v);
    }

    #[tokio::test]
    async fn embedder_without_provider_is_local() {
        let embedder = Embedder::new(None);
        assert_eq!(embedder.provider_id(), LOCAL_PROVIDER);
        let out = embedder.embed("hello world").await;
        assert_eq!(out.provider_id, LOCAL_PROVIDER);
        assert_eq!(out.values.len(), LOCAL_DIMENSIONS);
    }
}
