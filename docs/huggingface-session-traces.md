# Export a session for Hugging Face

The `roder-api` example `export_sts` converts one thread's `events.jsonl` into
the [Session Trace Simple Format](https://huggingface.co/docs/hub/session-traces-format)
(STS). Run it from a Roder source checkout:

```sh
cargo run -p roder-api --example export_sts -- /path/to/thread/events.jsonl THREAD_ID > session.jsonl
```

The input is the JSONL thread store's event log, not `item_events.jsonl` or a
database dump. Supply the thread ID recorded in the log. Use a stopped thread
or a consistent copy of its log. This example reads local files, writes JSONL
to stdout, and makes no network requests. It validates the complete input before
writing output; shell redirection may still create an empty file on failure.

The header uses `harness: "roder"`. Hugging Face documents generic STS rendering,
but Roder's icon and label have not been registered or verified in the hosted
viewer. This exporter does not establish native support for raw Roder logs.

## What is exported

| Roder transcript item | STS representation |
| --- | --- |
| User text | `role: "user"`, `content` |
| Assistant text | `role: "assistant"`, `content` |
| Reasoning summary | Assistant message with `reasoningContent` |
| Tool call | Assistant `toolCalls`, preserving ID, name, and JSON argument string |
| Tool result | `role: "tool"`, `toolCallId`, unchanged result text |
| Append timestamp | Epoch milliseconds in `timestamp` |

The exporter preserves event order and emits separate messages for separate
transcript items. It does not infer model names, system prompts, or unavailable
message content from provider metadata. Assistant phase labels and tool result
names are not carried; tool names come from the linked calls. Lifecycle events,
streaming deltas, token usage, and other non-transcript events are ignored.

Images, tool display payloads, tool error flags, file changes, compaction
records, transcript errors, and provider metadata have no documented STS
mapping. The default export rejects them. To export just the representable text:

```sh
cargo run -p roder-api --example export_sts -- /path/to/thread/events.jsonl THREAD_ID --allow-lossy > session.jsonl
```

This option omits those fields and unmapped transcript items. Tool error text is
kept, but the viewer cannot distinguish its error flag. Reasoning summaries stay
in the export. There is no automatic secret redaction. Review the resulting file
before sharing it; prompts and tool output may contain private code or credentials.

An append without an embedded item is always rejected, including in lossy mode.
Mixed thread IDs, invalid JSON, duplicate event IDs, non-increasing sequence
numbers, duplicate tool IDs/results, and tool results without preceding calls
also fail. A call without a result is allowed for interrupted sessions. Forked
logs that reference calls in an unavailable parent cannot be exported by this
example. This is a single event-log converter, not a full session-store exporter.

## Synthetic example and checks

`crates/roder-api/examples/fixtures/sts-events.jsonl` is an invented five-item
session. `sts-expected.jsonl` contains its expected STS output. Neither fixture
contains a real user's trace. Run:

```sh
cargo test -p roder-api --example export_sts
cargo run -p roder-api --example export_sts -- crates/roder-api/examples/fixtures/sts-events.jsonl synthetic-sts
```

The upstream schema is currently the prose contract in
[`huggingface/hub-docs`](https://github.com/huggingface/hub-docs/blob/main/docs/hub/session-traces-format.md).
These checks establish local conformance to its documented fields; a hosted
viewer check and Roder branding registration remain separate work.
