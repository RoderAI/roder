use std::io::BufRead;

use anyhow::{Context, bail};

use crate::format::ApiTranscriptRecord;

pub fn read_jsonl_records(reader: impl BufRead) -> anyhow::Result<Vec<ApiTranscriptRecord>> {
    let mut records = Vec::new();
    let mut last_seq = None;
    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = line.with_context(|| format!("read transcript line {line_number}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let record: ApiTranscriptRecord = serde_json::from_str(&line)
            .with_context(|| format!("parse transcript line {line_number}"))?;
        record
            .validate()
            .with_context(|| format!("validate transcript line {line_number}"))?;
        if let Some(seq) = record.seq() {
            if let Some(previous) = last_seq
                && seq <= previous
            {
                bail!(
                    "transcript line {line_number} has non-monotonic seq {seq}; previous seq was {previous}"
                );
            }
            last_seq = Some(seq);
        }
        records.push(record);
    }
    Ok(records)
}

pub struct ApiTranscriptReader<R> {
    reader: R,
}

impl<R: BufRead> ApiTranscriptReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub fn read_records(self) -> anyhow::Result<Vec<ApiTranscriptRecord>> {
        read_jsonl_records(self.reader)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;

    use crate::format::RecordedArtifactRef;

    use super::*;

    #[test]
    fn reader_rejects_non_monotonic_sequences() {
        let one = serde_json::to_string(&event(2)).unwrap();
        let two = serde_json::to_string(&event(2)).unwrap();
        let err = read_jsonl_records(Cursor::new(format!("{one}\n{two}\n")))
            .unwrap_err()
            .to_string();

        assert!(err.contains("non-monotonic seq 2"), "{err}");
    }

    #[test]
    fn reader_accepts_valid_jsonl_records() {
        let line = serde_json::to_string(&event(1)).unwrap();

        let records = read_jsonl_records(Cursor::new(format!("{line}\n"))).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(serde_json::to_value(&records[0]).unwrap()["seq"], json!(1));
    }

    fn event(seq: u64) -> ApiTranscriptRecord {
        ApiTranscriptRecord::Artifact {
            seq,
            at_ms: 0,
            artifact: RecordedArtifactRef {
                path: format!("artifact-{seq}.txt"),
                media_type: "text/plain".to_string(),
                sha256: Some(format!("{seq:x}")),
                bytes: Some(seq),
            },
        }
    }
}
