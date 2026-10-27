use anyhow::ensure;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(super) struct Cursor {
    pub version: u8,
    pub thread: String,
    pub turn: String,
    pub source: i64,
    pub item: String,
}

pub(super) fn encode(cursor: &Cursor) -> anyhow::Result<String> {
    Ok(serde_json::to_vec(cursor)?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

pub(super) fn decode(value: &str) -> anyhow::Result<Cursor> {
    ensure!(
        value.len() <= 16384 && value.len().is_multiple_of(2),
        "invalid history cursor"
    );
    let bytes: Result<Vec<_>, _> = value
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .ok_or_else(|| anyhow::anyhow!("invalid history cursor"))
        })
        .collect();
    let cursor: Cursor = serde_json::from_slice(&bytes?)?;
    ensure!(cursor.version == 1, "unsupported history cursor version");
    Ok(cursor)
}
