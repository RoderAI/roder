use sha2::{Digest, Sha256};

pub const DEFAULT_FAKE_DIMENSIONS: usize = 32;

pub fn fake_embedding(text: &str) -> Vec<f32> {
    let mut values = vec![0.0_f32; DEFAULT_FAKE_DIMENSIONS];
    for token in text.split_whitespace().map(|token| token.to_lowercase()) {
        let digest = Sha256::digest(token.as_bytes());
        let idx = (digest[0] as usize) % DEFAULT_FAKE_DIMENSIONS;
        values[idx] += 1.0;
    }
    normalize(values)
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
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
    fn vector_scores_related_text_higher() {
        let query = fake_embedding("rust memory sqlite");
        let related = fake_embedding("sqlite memory store");
        let unrelated = fake_embedding("terminal media generation");
        assert!(cosine(&query, &related) > cosine(&query, &unrelated));
    }
}
