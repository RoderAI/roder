//! Export one JSONL thread event log to Hugging Face's Session Trace Simple Format.
//! See docs/huggingface-session-traces.md for scope and limitations.

use std::collections::HashSet;
use std::io::{BufRead, Write};

use anyhow::{Context, bail, ensure};
use roder_api::events::{EventEnvelope, RoderEvent};
use roder_api::transcript::TranscriptItem;
use serde_json::{Value, json};

fn export(reader: impl BufRead, thread_id: &str, allow_lossy: bool) -> anyhow::Result<Vec<Value>> {
    ensure!(!thread_id.trim().is_empty(), "thread id must not be empty");
    let mut output = vec![json!({"type":"session", "harness":"roder", "id":thread_id})];
    let mut calls = HashSet::new();
    let mut results = HashSet::new();
    let mut event_ids = HashSet::new();
    let mut previous_seq = None;
    for (line_number, line) in reader.lines().enumerate() {
        let line = line?;
        // The store also accepts concatenated records on a line.
        let records = serde_json::Deserializer::from_str(&line).into_iter::<EventEnvelope>();
        for record in records {
            let envelope = record.with_context(|| format!("event log line {}", line_number + 1))?;
            ensure!(
                envelope.thread_id.as_deref() == Some(thread_id),
                "event belongs to another thread"
            );
            ensure!(event_ids.insert(envelope.event_id), "duplicate event id");
            ensure!(
                previous_seq.is_none_or(|seq| envelope.seq > seq),
                "event sequence must increase"
            );
            previous_seq = Some(envelope.seq);
            let RoderEvent::TranscriptItemAppended(event) = envelope.event else {
                continue;
            };
            ensure!(
                event.thread_id == thread_id,
                "transcript item belongs to another thread"
            );
            let item = event
                .item
                .context("transcript append has no embedded item; cannot reconstruct this log")?;
            let mut message = match item {
                TranscriptItem::UserMessage(user) => {
                    ensure!(
                        allow_lossy || user.images.is_empty(),
                        "user images require --allow-lossy"
                    );
                    json!({"role":"user", "content":user.text})
                }
                TranscriptItem::AssistantMessage(assistant) => {
                    json!({"role":"assistant", "content":assistant.text})
                }
                TranscriptItem::ReasoningSummary(reasoning) => {
                    json!({"role":"assistant", "content":"", "reasoningContent":reasoning.text})
                }
                TranscriptItem::ToolCall(call) => {
                    ensure!(
                        !call.id.is_empty() && calls.insert(call.id.clone()),
                        "empty or duplicate tool call id"
                    );
                    ensure!(!call.name.is_empty(), "empty tool name");
                    serde_json::from_str::<Value>(&call.arguments)
                        .context("tool arguments must contain JSON")?;
                    json!({"role":"assistant", "content":"", "toolCalls":[{
                        "id":call.id, "function":{"name":call.name, "arguments":call.arguments}
                    }]})
                }
                TranscriptItem::ToolResult(result) => {
                    ensure!(
                        calls.contains(&result.id),
                        "tool result has no preceding call"
                    );
                    ensure!(results.insert(result.id.clone()), "duplicate tool result");
                    ensure!(
                        allow_lossy || result.display_payload.is_none(),
                        "tool display payload requires --allow-lossy"
                    );
                    // STS has no error flag. Keep the result text intact and require
                    // explicit consent when that flag cannot be represented.
                    ensure!(
                        allow_lossy || !result.is_error,
                        "tool error flag requires --allow-lossy"
                    );
                    json!({"role":"tool", "content":result.result, "toolCallId":result.id})
                }
                TranscriptItem::FileChange(_)
                | TranscriptItem::ContextCompaction(_)
                | TranscriptItem::Error(_)
                | TranscriptItem::ProviderMetadata(_) => {
                    ensure!(
                        allow_lossy,
                        "transcript item has no STS mapping; use --allow-lossy to omit it"
                    );
                    continue;
                }
            };
            let millis = event.timestamp.unix_timestamp_nanos().div_euclid(1_000_000);
            message["timestamp"] = json!(i64::try_from(millis)?);
            output.push(json!({"type":"message", "message":message}));
        }
    }
    ensure!(
        output.len() > 1,
        "event log contains no exportable transcript items"
    );
    Ok(output)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(args.len() == 2 || (args.len() == 3 && args[2] == "--allow-lossy")) {
        bail!("usage: export_sts <events.jsonl> <thread-id> [--allow-lossy]");
    }
    let input = std::fs::File::open(&args[0]).context("open event log")?;
    // Validate the entire input before writing any trace bytes to stdout.
    let output = export(std::io::BufReader::new(input), &args[1], args.len() == 3)?;
    if args.len() == 3 {
        eprintln!(
            "Lossy text export: images, display payloads, error flags, and unmapped transcript items are omitted."
        );
    }
    let mut stdout = std::io::stdout().lock();
    for record in output {
        serde_json::to_writer(&mut stdout, &record)?;
        writeln!(stdout)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("fixtures/sts-events.jsonl");

    #[test]
    fn synthetic_tool_session_matches_documented_sts_shape() {
        let actual = export(FIXTURE.as_bytes(), "synthetic-sts", false).unwrap();
        let expected: Vec<Value> = include_str!("fixtures/sts-expected.jsonl")
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_corruption_and_mixed_sessions() {
        for input in [
            FIXTURE.replace("synthetic-sts", "other"),
            format!("{FIXTURE}{}", FIXTURE.lines().next().unwrap()),
            format!("{FIXTURE}{{"),
            FIXTURE.replace("\"seq\":2", "\"seq\":0"),
            FIXTURE.replace(
                "\"item\":{\"UserMessage\":{\"text\":\"Say hello\"}}",
                "\"item\":null",
            ),
        ] {
            assert!(
                export(input.as_bytes(), "synthetic-sts", false).is_err(),
                "{input}"
            );
        }
        assert!(export("".as_bytes(), "synthetic-sts", false).is_err());
    }

    #[test]
    fn rejects_unlinked_results_and_invalid_arguments() {
        let input = FIXTURE.replace(
            "\"id\":\"call-1\",\"name\":null",
            "\"id\":\"missing\",\"name\":null",
        );
        assert!(export(input.as_bytes(), "synthetic-sts", false).is_err());
        let input = FIXTURE.replace("\"arguments\":\"{}\"", "\"arguments\":\"invalid\"");
        assert!(export(input.as_bytes(), "synthetic-sts", false).is_err());
    }

    #[test]
    fn lossy_export_requires_explicit_consent() {
        for input in [
            FIXTURE.replace("\"is_error\":false", "\"is_error\":true"),
            FIXTURE.replace("\"text\":\"Say hello\"", "\"text\":\"Say hello\",\"images\":[{\"image_url\":\"https://example.com/image.png\"}]"),
            FIXTURE.replace("\"is_error\":false", "\"is_error\":false,\"display_payload\":{\"path\":\"example.txt\"}"),
            FIXTURE.replace("\"AssistantMessage\":{\"text\":\"Hello!\"}", "\"FileChange\":{\"path\":\"example.txt\",\"change_type\":\"create\"}"),
        ] {
            assert!(export(input.as_bytes(), "synthetic-sts", false).is_err());
            assert!(export(input.as_bytes(), "synthetic-sts", true).is_ok());
        }
    }
}
