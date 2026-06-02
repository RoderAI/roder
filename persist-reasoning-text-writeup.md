# `codex/persist-reasoning-text` — changes writeup

## The replayability problems

The branch started from a runtime that could *stream* a turn but couldn't *replay* one faithfully. Three concrete failures sat under that:

**1. The on-disk log was split, and the two halves could disagree.**

A turn was written to two files. `events.jsonl` held the turn lifecycle (started / completed / failed / interrupted). `transcript_items.jsonl` held the items the user and the agent had produced. Each item line was a thin triple — `{ turn_id, timestamp, item }` — with no event id, no sequence number, no kind tag, no source. To rebuild a turn you read both files and stitched them: items from one, completion and usage from the other. A crash between the two appends left the thread in a state where the lifecycle said the turn had three items but the item log only had two, or vice versa. There was no way to detect the drift after the fact, because the item lines weren't sequenced.

**2. Streaming state was visible on the wire but not on disk.**

Partial reasoning text, in-progress agent messages, and "tool is running" markers existed only as JSON-RPC notifications going out of the app server. They were wire concerns; nothing about them was durable. If you killed the process mid-turn and resumed, the runtime had no way to rehydrate the partial state, because the partial state had never been an event — only a side effect.

**3. The wire shape had leaked into the core's event bus.**

The runtime was emitting protocol-shaped events on its own internal bus — variants like `ThreadItemEventRecorded` that carried wire identity. That meant the runtime couldn't change without also moving the wire format, and the protocol crate couldn't evolve without rippling into core. There was a presentation layer hiding inside the runtime, and it was load-bearing.

## The solutions, and where strong typing did the work

The fix is one disk artifact, two streams, and a typed envelope on every line.

**`events.jsonl` is now the single source of truth for the turn timeline.** `transcript_items.jsonl` is gone. The `TranscriptItemAppended` variant of `RoderEvent` carries the transcript item inline as `Option<TranscriptItem>` — the `Option` is documented (the `None` case is the metadata-only append). Every line on disk is now a full `EventEnvelope`:

```
{ event_id, seq, source, kind, thread_id, turn_id, event: <typed variant> }
```

That envelope is what makes events replayable in a strong sense — each one is addressable (`event_id`), orderable (`seq`), attributable (`source`), self-describing (`kind`), and *typed* at the variant level. The projector doesn't sniff JSON; it pattern-matches against `RoderEvent`. If a new event kind appears, the compiler tells every projection site about it.

**Streaming state is now a first-class durable stream of its own.** `item_events.jsonl` sits alongside `events.jsonl` and holds the canonical typed item events. These aren't a wire format that happens to be persisted — they're a domain stream with its own envelope (`ThreadItemEvent`), its own sequence space, and its own typed payload (`ThreadItemEventKind`: `ItemStarted` / `ItemDelta` / `ItemCompleted`). Deltas carry typed bodies (`ThreadItemDelta::AgentMessageText { delta, phase }`, `ThreadItemDelta::ReasoningText { delta, content_index }`, the two `ReasoningSummary` variants). Item status is a three-state enum (`ThreadItemStatus::{InProgress, Completed, Failed}`), not an untyped string.

The typing pays its keep in the projector. `project_thread_item_events` is a fold: for each event, match on the kind, dispatch to `apply_thread_item_delta`, which itself matches on `(item, delta)` pairs. The match arms are exhaustive — adding a delta variant or a `ThreadItem` variant forces the projector to handle it. The old shape couldn't have been folded like this: it would have needed runtime conditionals on string fields, and a forgotten string would just silently get dropped.

**Presentation has been evicted from the runtime.** The `ThreadItemEventRecorded` variant on the internal bus is gone. The runtime now produces typed item events and hands them to a *projector service* (`record_thread_item_event_kind`). The wire mapping — converting runtime inference events into `ThreadItemEventKind`s with the right item ids and lifecycle — lives in `crates/roder-app-server/src/item_stream.rs`. The runtime no longer knows what a wire frame is.

## New concepts

- **`ThreadItem`** — the domain-level item enum: `UserMessage`, `AgentMessage`, `Reasoning`, `ToolExecution`, `Compaction`, `Error`, `Raw`. Distinct from `TranscriptItem` (which is the raw transcript shape); `ThreadItem` is what an item looks like with streaming and status attached.
- **`ThreadItemDelta`** — typed partial updates. Each variant declares exactly which slot it updates and how, so the projector can apply them with no string-sniffing.
- **`ThreadItemEventKind`** — the `Started` / `Delta` / `Completed` lifecycle.
- **`ThreadItemEvent`** — the envelope around the kind: `seq`, `event_id`, `thread_id`, `turn_id`, `timestamp`, `event`.
- **`ThreadItemStatus`** — `InProgress` / `Completed` / `Failed`, replacing free-form status strings.
- **`ThreadItemTurnRecord`** — the projection output: a per-turn bundle of materialized `ThreadItem`s rebuilt from the event stream.
- **`ThreadItemCache`** (in `roder-core`) — a bounded (256-thread LRU) per-thread cache of `next_item_event_seq`, the set of known `(turn_id, item_id)` pairs, and per-turn transcript item counts. This is the thing that eliminated the full-log re-reads on the append path: the runtime no longer asks the store "what was the last seq" before writing — it asks the cache.
- **`project_turns_from_events`** and **`project_thread_item_events`** — the two public projection functions on `roder-api::thread::projection`. They are how an extension turns a raw event log back into the materialized snapshot, without depending on any runtime internals.

## How the architectural guard-rails held

The branch keeps the four-crate boundary the project has been pulling toward, and arguably tightens it.

**`roder-api`** owns the domain. The new item types live here. The projection helpers live here. There is nothing in `api` that knows about a JSON-RPC notification, a JSONL file, or a runtime task. The `extension_api_compat` and `thread_projection` tests pin this — any extension author can write a new `ThreadStore` against `api` alone and rebuild a thread snapshot from the event log without reading runtime code.

**`roder-core`** owns the runtime. It consumes `api`, never `protocol`. The `ThreadItemCache` is correctly placed: it's runtime-side bookkeeping, not a domain type, and lives in core. When core needs to emit an item event it produces an `api` type and hands it to a service; the wire transformation happens above.

**`roder-protocol`** owns wire shapes. It has its own `Item`, `ThreadItemEvent`, `ThreadItemEventKind`, `ThreadItemDelta`, `ThreadItemStatus` — not re-exports — with `From<roder_api::thread::*>` impls bridging them. The wire shape can diverge from the domain shape (rename fields, restructure variants, add wire-only fields) without anyone in `api` or `core` knowing.

**`roder-app-server`** owns the translation. `item_stream.rs` is the only place that converts runtime inference events into wire frames. It owns the bridging logic in both directions: producing notifications for clients, and producing typed item events to be persisted. When persistence fails it now surfaces the error (`fix(app-server): surface item stream persistence failures`) rather than swallowing it.

**`roder-ext-jsonl-thread-store`** is back to being just an implementation. It used to hold the projector privately; that's now in `api`. Its `store.rs` lost ~60 lines and got simpler. The store calls `project_turns_from_events` like any other consumer would.

The contract you asked about earlier — *I can write an extension against `api` and a client against `protocol` without reading the runtime* — now holds in both directions. The test name `extension_authors_can_project_turns_from_raw_events` is the explicit assertion of that contract; if anyone ever tries to drag the projector back inside an extension or into core, that test fails and tells them why.

## What this leaves us with

A single append-only log per thread, with typed envelopes on every line, in two streams that share envelope structure but carry different domain payloads. A fold-based projector that the compiler keeps honest. A clean three-stage flow — `runtime → app-server → wire` — with no back-channels. And the property the branch was named for: enough state on disk that a partially-completed turn, including its in-flight reasoning, can be reconstructed on resume.

The one substantive thing still outstanding is the migration story for threads written *before* this branch — their `events.jsonl` rows don't carry the `item` field on `TranscriptItemAppended`, and `transcript_items.jsonl` is no longer read. Old threads will load with empty turns until that gap is closed.
