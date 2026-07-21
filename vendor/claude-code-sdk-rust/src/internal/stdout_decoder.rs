use std::collections::VecDeque;

use serde_json::Value;

use crate::error::{CLIJSONDecodeError, Result};

#[derive(Debug)]
pub(crate) struct StdoutDecoder {
    buffer: String,
    pending: VecDeque<Vec<u8>>,
    max_buffer_size: usize,
}

impl StdoutDecoder {
    pub(crate) fn new(max_buffer_size: usize) -> Self {
        Self {
            buffer: String::new(),
            pending: VecDeque::new(),
            max_buffer_size,
        }
    }

    pub(crate) fn push(&mut self, chunk: &str) -> Result<()> {
        self.buffer.push_str(chunk);
        self.drain()
    }

    pub(crate) fn next(&mut self) -> Option<Vec<u8>> {
        self.pending.pop_front()
    }

    pub(crate) fn finish(&mut self) -> Result<()> {
        self.drain()?;
        let trimmed = self.buffer.trim();
        if trimmed.is_empty() || !trimmed.starts_with('{') {
            self.buffer.clear();
            return Ok(());
        }

        match serde_json::from_str::<Value>(trimmed) {
            Ok(_) => self.drain(),
            Err(error) => Err(CLIJSONDecodeError::new(trimmed, error).into()),
        }
    }

    fn drain(&mut self) -> Result<()> {
        loop {
            if self.buffer.len() > self.max_buffer_size {
                return Err(CLIJSONDecodeError::new(
                    format!(
                        "JSON message exceeded maximum buffer size of {} bytes",
                        self.max_buffer_size
                    ),
                    serde_json::from_str::<Value>("").unwrap_err(),
                )
                .into());
            }

            let Some(start) = self.buffer.find(|ch: char| !ch.is_whitespace()) else {
                self.buffer.clear();
                return Ok(());
            };

            if !self.buffer[start..].starts_with('{') {
                let Some(newline) = self.buffer[start..].find('\n') else {
                    if start > 0 {
                        self.buffer.drain(..start);
                    }
                    return Ok(());
                };
                self.buffer.drain(..start + newline + 1);
                continue;
            }

            let trimmed = &self.buffer[start..];
            let mut stream = serde_json::Deserializer::from_str(trimmed).into_iter::<Value>();
            match stream.next() {
                Some(Ok(_)) => {
                    let offset = stream.byte_offset();
                    let json = trimmed[..offset].trim().as_bytes().to_vec();
                    if json.len() > self.max_buffer_size {
                        return Err(CLIJSONDecodeError::new(
                            format!(
                                "JSON message exceeded maximum buffer size of {} bytes",
                                self.max_buffer_size
                            ),
                            serde_json::from_str::<Value>("").unwrap_err(),
                        )
                        .into());
                    }
                    self.pending.push_back(json);
                    self.buffer.drain(..start + offset);
                }
                Some(Err(error)) if error.is_eof() => {
                    if start > 0 {
                        self.buffer.drain(..start);
                    }
                    return Ok(());
                }
                Some(Err(error)) => {
                    return Err(CLIJSONDecodeError::new(trimmed, error).into());
                }
                None => return Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StdoutDecoder;

    #[test]
    fn parses_multiple_json_objects_from_one_chunk() {
        let mut decoder = StdoutDecoder::new(1024);
        decoder
            .push("{\"type\":\"message\",\"id\":\"msg1\"}\n\n{\"type\":\"result\",\"id\":\"res1\"}")
            .unwrap();

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&decoder.next().unwrap()).unwrap()["id"],
            "msg1"
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&decoder.next().unwrap()).unwrap()["id"],
            "res1"
        );
        assert!(decoder.next().is_none());
    }

    #[test]
    fn parses_json_split_across_chunks() {
        let mut decoder = StdoutDecoder::new(1024);
        decoder.push(r#"{"type":"assistant","#).unwrap();
        assert!(decoder.next().is_none());
        decoder.push(r#""message":{"content":[]}}"#).unwrap();

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&decoder.next().unwrap()).unwrap()["type"],
            "assistant"
        );
    }

    #[test]
    fn skips_non_json_debug_lines() {
        let mut decoder = StdoutDecoder::new(1024);
        decoder
            .push("[SandboxDebug] ignored\n   \n{\"type\":\"result\"}\n")
            .unwrap();

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&decoder.next().unwrap()).unwrap()["type"],
            "result"
        );
    }

    #[test]
    fn enforces_max_buffer_size_for_incomplete_json() {
        let mut decoder = StdoutDecoder::new(8);
        let err = decoder.push("{\"data\":\"too long").unwrap_err();

        assert!(err.to_string().contains("exceeded maximum buffer size"));
    }
}
