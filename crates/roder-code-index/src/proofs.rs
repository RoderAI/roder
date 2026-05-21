use roder_api::code_index::{CodeChunk, ContentProof, MerkleHash};

pub fn proof_for_chunk(
    workspace_root_hash: impl Into<MerkleHash>,
    generation_id: impl Into<String>,
    chunk: &CodeChunk,
) -> ContentProof {
    ContentProof {
        path_hash: chunk.path_hash.clone(),
        content_hash: chunk.content_hash.clone(),
        workspace_root_hash: workspace_root_hash.into(),
        generation_id: generation_id.into(),
    }
}

pub fn verify_chunk_proof(
    proof: &ContentProof,
    expected_workspace_root_hash: &str,
    chunk: &CodeChunk,
) -> bool {
    proof.workspace_root_hash == expected_workspace_root_hash
        && proof.path_hash == chunk.path_hash
        && proof.content_hash == chunk.content_hash
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use roder_api::code_index::{CodeByteRange, CodeLineRange};

    use super::*;

    #[test]
    fn proofs_verify_chunk_possession() {
        let chunk = CodeChunk {
            chunk_hash: "chunk".to_string(),
            path: PathBuf::from("src/lib.rs"),
            path_hash: "path".to_string(),
            byte_range: CodeByteRange { start: 0, end: 10 },
            line_range: CodeLineRange { start: 1, end: 1 },
            content_hash: "content".to_string(),
            language: Some("rust".to_string()),
            symbol_hint: Some("lib".to_string()),
        };
        let proof = proof_for_chunk("root", "gen", &chunk);

        assert!(verify_chunk_proof(&proof, "root", &chunk));

        let mut mismatched = chunk.clone();
        mismatched.content_hash = "other".to_string();
        assert!(!verify_chunk_proof(&proof, "root", &mismatched));
    }
}
