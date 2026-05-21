use std::io::Write;

use anyhow::Context;

use crate::format::ApiTranscriptRecord;

pub fn write_jsonl_record(
    writer: &mut impl Write,
    record: &ApiTranscriptRecord,
) -> anyhow::Result<()> {
    record.validate()?;
    serde_json::to_writer(&mut *writer, record).context("serialize api transcript record")?;
    writer
        .write_all(b"\n")
        .context("write api transcript newline")?;
    Ok(())
}

pub struct ApiTranscriptWriter<W> {
    writer: W,
}

impl<W: Write> ApiTranscriptWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    pub fn write_record(&mut self, record: &ApiTranscriptRecord) -> anyhow::Result<()> {
        write_jsonl_record(&mut self.writer, record)
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use crate::format::{
        ApiTranscriptHeader, ApiTranscriptRecord, RecordedTerminalSize, SUPPORTED_SCHEMA_VERSION,
    };

    use super::*;

    #[test]
    fn writer_emits_one_json_record_per_line() {
        let mut bytes = Vec::new();
        let mut writer = ApiTranscriptWriter::new(&mut bytes);

        writer
            .write_record(&ApiTranscriptRecord::Header(ApiTranscriptHeader {
                schema_version: SUPPORTED_SCHEMA_VERSION,
                created_at: OffsetDateTime::UNIX_EPOCH,
                roder_version: "dev".to_string(),
                cwd: "<redacted>".to_string(),
                terminal: RecordedTerminalSize { cols: 80, rows: 24 },
                features: Vec::new(),
                metadata: serde_json::Value::Null,
            }))
            .unwrap();

        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("\"kind\":\"header\""));
    }
}
