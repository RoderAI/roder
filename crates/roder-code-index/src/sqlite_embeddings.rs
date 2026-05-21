use roder_api::code_index::{ChunkEmbedding, CodeChunk};
use rusqlite::{Connection, OptionalExtension, params};

use crate::hex_sha256;

const PROVIDER_ID: &str = "fake-code-index";
const MODEL_ID: &str = "fake-code-vector-16";

pub(crate) fn ensure_embedding(
    conn: &Connection,
    chunk: &CodeChunk,
) -> anyhow::Result<(ChunkEmbedding, bool)> {
    if let Some(embedding) = load_embedding(conn, &chunk.content_hash)? {
        return Ok((embedding_for_chunk(chunk, embedding.vector), true));
    }

    let vector = fake_embedding(&chunk.content_hash);
    conn.execute(
        "INSERT INTO embedding_cache(content_hash, vector_json, provider, model, dimensions)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            chunk.content_hash,
            serde_json::to_string(&vector)?,
            PROVIDER_ID,
            MODEL_ID,
            vector.len() as i64,
        ],
    )?;
    Ok((embedding_for_chunk(chunk, vector), false))
}

fn load_embedding(conn: &Connection, content_hash: &str) -> anyhow::Result<Option<ChunkEmbedding>> {
    conn.query_row(
        "SELECT vector_json, provider, model, dimensions FROM embedding_cache WHERE content_hash = ?1",
        params![content_hash],
        |row| {
            let vector_json: String = row.get(0)?;
            let vector: Vec<f32> = serde_json::from_str(&vector_json).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(err),
                )
            })?;
            Ok(ChunkEmbedding {
                chunk_hash: String::new(),
                provider: row.get(1)?,
                model: row.get(2)?,
                dimensions: row.get::<_, i64>(3)? as usize,
                vector,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn embedding_for_chunk(chunk: &CodeChunk, vector: Vec<f32>) -> ChunkEmbedding {
    ChunkEmbedding {
        chunk_hash: chunk.chunk_hash.clone(),
        provider: PROVIDER_ID.to_string(),
        model: MODEL_ID.to_string(),
        dimensions: vector.len(),
        vector,
    }
}

fn fake_embedding(seed: &str) -> Vec<f32> {
    let bytes = hex_sha256(seed);
    bytes
        .as_bytes()
        .chunks(4)
        .take(16)
        .map(|chunk| {
            let sum = chunk.iter().fold(0u32, |acc, byte| acc + (*byte as u32));
            (sum % 1000) as f32 / 1000.0
        })
        .collect()
}
