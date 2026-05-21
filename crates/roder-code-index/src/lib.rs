pub mod chunk;
pub mod merkle;
pub mod proofs;
pub mod sqlite;
pub(crate) mod sqlite_embeddings;
pub(crate) mod sqlite_schema;

pub(crate) fn hex_sha256(bytes: impl AsRef<[u8]>) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes.as_ref());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}
