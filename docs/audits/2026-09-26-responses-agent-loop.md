# Responses agent loop audit — 26 September 2026

## Summary

There are concrete correctness fixes and useful improvements to take from the current Codex reference. The July tool-parity work is implemented; the highest-value work now is SSE correctness, the current client tool-search contract, and recovery during interactive turns.

Eight targeted contract probes reproduce defects or missing behavior. The existing 83 Responses unit tests and 15 agent-loop integration tests pass. Production code was not changed by this audit.

## Scope and baseline

| Source | Audited snapshot |
| --- | --- |
| Roder | `816974be81b5cb7a1d249c14bc5bd26d7afb14ff`, September 26 |
| Local Codex reference, `/Users/pz/w/codex` | `8b78f4796605bda8e31329537f5fef036e405e66`, September 25 |
| Earlier documented parity work | `roadmap/110-roder-codex-parity-native-loop.md`, July 7 audit; implementation commit `417017ab`, July 14 |

The July audit is the prior review located in the repository, rather than an assertion about the date of our last conversation. Codex was inspected locally; its remote was not refreshed and its test suite was not run. No live model calls or paid benchmark runs were made. Unrelated local work was preserved.

The comparison covers request mapping, streaming, tool-search continuation, retry/error behavior, steering, tool dispatch, history replay, and compaction. The Codex subscription provider shares the same Roder Responses engine: `crates/roder-extension-host/src/lib.rs:1130` constructs `OpenAiResponsesEngine` for the ChatGPT backend.

## What is already improved since the July audit

| Earlier issue | Current implementation |
| --- | --- |
| No path-based image tool | `view_image` is registered and its output is forwarded as Responses `input_image`; provider unit coverage passes. |
| Tool failure limit stops eval iteration | `continue_on_failure_limit` and bounded empty-tool-call nudges exist; eval defaults enable the persistence knobs. |
| No persistent Codex-style exec wrapper | `unified_exec` is implemented over the exec session manager. |
| JSON-only patches | The custom call, parsing, and output replay paths exist, but their provider/model gates leave a substantial gap described below. |
| Loss of structured provider state | Function/custom calls, encrypted reasoning, and provider compaction items are replayed. Compaction boundaries emitted before a broken stream are persisted immediately. |

The draft status and several claims in `roadmap/110` / `roadmap/STATUS.md` are stale relative to the code and July 14 commit. They should not drive another implementation of the completed work. The July commit describes successful benchmark A/B results; those results were not independently rerun in this audit.

## Findings

### 1. P1 — Decode SSE as a byte stream without corrupting UTF-8

**Roder:** `crates/roder-ext-openai-responses/src/provider.rs:1219`–`1221` applies `String::from_utf8_lossy` independently to every HTTP chunk, then concatenates the strings.

HTTP chunks can split a multibyte character. The lossy conversion permanently replaces each incomplete fragment before the rest arrives. This affects assistant output and string-valued tool arguments, including filenames and patch contents.

**Reproduced:** sending the bytes of `café` with a chunk boundary between `C3` and `A9` produces `caf��` through the actual HTTP/SSE parser.

**Codex reference:** `codex-rs/codex-api/src/sse/responses.rs:600` uses `eventsource_stream` to parse the byte stream.

**Recommended change:** adopt a streaming SSE decoder, or buffer bytes until complete UTF-8 sequences and SSE frames are available. Add HTTP fixtures splitting Unicode in text, function arguments, and custom patch input. Also bound unfinished frame buffering.

### 2. P1 — Treat `response.completed` as terminal without waiting for HTTP EOF

**Roder:** the provider loop at `provider.rs:1219` continues reading after `state.terminal` becomes true. The core loop at `crates/roder-core/src/runtime.rs:4612` records completion metadata but keeps polling until the stream ends.

A provider or proxy can emit a complete response and keep the HTTP body open. Roder then delays tool execution and turn completion. A later read timeout can convert an already completed response into a failed turn or an eval retry. Continued keepalive bytes can prolong the delay.

**Reproduced:** after receiving `Completed`, another poll fails to reach EOF within 100 ms when the fixture keeps HTTP open for 500 ms.

**Codex reference:** `codex-api/src/sse/responses.rs:700` returns immediately after publishing `Completed`; `core/src/session/turn.rs:2953` exits the sampling loop on completion.

**Recommended change:** finish the provider response at the terminal event, then explicitly run any pending tool-search continuation. Have the core require one terminal outcome for each inference step. Test completion followed by open HTTP, trailing heartbeats, and a transport failure; completion must remain successful.

### 3. P1 — Update client tool search to the current wire contract

**Roder:** `provider.rs:1373` recognizes client searches by top-level `query` / `queries` and missing `results`. Query extraction at `1282` reads only top-level `query`. The continuation at `1322` sends a string-valued `output` containing names/descriptions without parameter schemas.

**Codex reference:** `codex-rs/protocol/src/models.rs:1094` defines a call with `execution: "client"`, `call_id`, and `arguments: {query, limit}`. `core/src/tools/router.rs:266` parses that shape. `core/src/tools/context.rs:238` returns `tool_search_output` with `status`, `execution`, and a `tools` array of full loadable tool specs.

**Reproduced:** the exact call shape from Codex's `tool_search_call_roundtrips` test is not recognized as client-executed by Roder. Its current fixtures use a different shape and therefore miss this failure.

**Impact:** a client search can be treated as a hosted completion without executing the local search or submitting its required output. The current continuation output also differs from the reference contract.

**Recommended change:** implement the current typed call/output contract and test fixtures derived from Codex's protocol shapes. Respect `arguments.limit`, return complete tool definitions, preserve call IDs, and persist search calls/results for subsequent requests. The replay whitelist at `provider.rs:2360` currently omits search call/output items. Enable this only for provider/model combinations whose endpoint contract is verified; native search is currently gated to the `openai` provider.

### 4. P1 — Fail explicitly when the client search continuation budget is exhausted

**Roder:** `provider.rs:1210` processes four responses through `0..=MAX_CLIENT_TOOL_SEARCH_ROUNDS`. On the last iteration it still sends the follow-up request at `1330`, then exits without consuming that response. Intermediate completions have been suppressed.

**Reproduced:** four consecutive search responses cause five HTTP requests, zero `Completed` events, zero errors, and normal stream EOF. The runtime's EOF branch accepts that stream end and can complete an empty turn.

**Recommended change:** check the continuation budget before issuing another request and return a typed exhaustion failure. Independently reject stream EOF without a canonical `Completed`/`Failed` outcome in the core. Test exact-budget completion, over-budget search, and no unconsumed final request.

### 5. P2 — Bring interactive stream recovery up to the reliability of eval runs

**Roder:** HTTP setup retries are shared, but the mid-stream retry at `runtime.rs:4452` is gated to `RuntimeProfile::Eval`. The retry classifier in `crates/roder-core/src/reliability.rs:77` recognizes a small set of error strings. SSE `response.failed` at `provider.rs:1633` becomes a message-only `InferenceFailure` and follows a separate terminal path even for retryable server failures.

**Reproduced:** an interactive fake provider that fails once with `stream closed before response.completed` and would succeed on its next invocation is called only once; the turn fails.

**Codex reference:** `core/src/session/turn.rs:1597` owns a sampling retry loop; `core/src/responses_retry.rs:62` applies typed error classification and transport fallback.

**Recommended change:** use one typed recovery policy across interactive/headless/eval profiles, with bounded per-request attempts. Preserve provider error code, status, request ID, response ID, and retry advice. Keep quota, authentication, invalid prompt, policy rejection, and Flex capacity failures distinct from retryable transport/server errors. Do not retry every `response.failed` indiscriminately.

Before expanding recovery, preserve completed response items and tool results as they arrive. Today most response content is staged until the stream ends, so a retry discards completed commentary/reasoning as well as unfinished output. Resume from committed history and avoid reexecuting tools with recorded results.

### 6. P2 — Honor server retry deadlines

**Roder:** `provider.rs:924` reads the error body but never captures `Retry-After`; `950` sleeps according to local policy only. `roder-api/src/reliability.rs:156` carries no server deadline. Stream error mapping also discards structured retry information.

**Reproduced:** a local HTTP 429 with `Retry-After: 1` and zero configured local backoff is retried in less than 1 ms.

**Recent Codex improvement:** commit `9d8de196748`, September 23, parses delay seconds and HTTP dates into a monotonic deadline captured when headers arrive. See `http-client/src/retry_after.rs:19`, `http-client/src/transport.rs:183`, and `core/src/responses_retry.rs:143`.

**Recommended change:** preserve and honor valid server advice through HTTP and stream recovery; otherwise use bounded jittered local backoff. Make the delay interruptible. Test both header forms, elapsed propagation time, expired advice, and terminal errors. Codex still has a TODO about respecting advice before a WebSocket-to-HTTP fallback; reproduce the tested behavior rather than copying that omission.

### 7. P2 — Emit one terminal failure per turn

**Roder:** a stream `Err` emits `TurnFailed` at `runtime.rs:4504`, then returns `Err` at `4521`. The outer task catches it and emits `TurnFailed` again at `3346`. Each emission also dispatches the local `Stop` hook.

**Reproduced:** one transport failure emits two `turn.failed` events for the same turn. The existing startup-error test stops at the first event and does not detect duplication.

**Recommended change:** give one layer ownership of the terminal outcome. Preserve error details and partial result state without emitting a second terminal lifecycle. Test startup failures, stream errors, explicit SSE failures, cancellation, and exactly-once Stop hook dispatch. This should be fixed alongside the retry changes.

### 8. P2 — Let steering preempt an unfinished sampling request

**Roder:** `runtime.rs:3755` queues steering input but sends no signal that wakes request setup, `stream.next()`, or retry backoff. Input is drained before the next model round or after a completed no-tool response.

**Reproduced:** an unfinished provider stream remains active after `steer_turn`; the new constraint does not reach a replacement inference request within the fixture's 100 ms window. The current steering integration test covers a blocked tool, not a blocked model stream.

**Recent Codex improvement:** commit `f92655d07f`, September 25, extends its `instant_interrupt` feature to preempt request setup, streaming, and retry backoff. `core/src/session/turn.rs:1598` watches user input; `2546`, `2609`, and `1696` apply preemption. Completed commentary is preserved, unfinished output is excluded, and WebSocket continuation state is reset.

**Recommended change:** add sampling-scoped preemption, separate from turn cancellation and tool cancellation. Wake blocked sampling/backoff on user steering, commit completed items, append the input, and resample in the same turn. Give mailbox delivery an explicit policy since cutting off a response's remaining tool calls changes agent coordination semantics. Verify app-server/ACP event behavior when implementing this.

### 9. P2 — Expand custom patch support to the actual Codex/new-model path

**Roder:** `provider.rs:640` supports freeform patches only for the `gpt-5.5` prefix. `654` additionally requires provider `openai`, excluding the `codex` subscription provider even with that model. The custom spec at `598` has no grammar; custom input deltas are not handled by the SSE event mapper.

**Codex reference:** `core/src/tools/handlers/apply_patch_spec.rs:9` advertises a freeform custom tool with a Lark grammar. `codex-api/src/sse/responses.rs:382` forwards `response.custom_tool_call_input.delta`. The handler creates a streaming patch diff consumer.

**Impact:** API `gpt-5.6`/`gpt-6-*` and Codex subscription sessions use the JSON patch tool despite the existing custom implementation. This is a confirmed mapping difference; a model-quality improvement has not been benchmarked here.

**Recommended change:** use explicit provider/model capabilities to advertise custom patches on supported current OpenAI and Codex models, add the patch grammar, and forward custom input deltas for progress/diff previews. Verify both API and subscription request bodies. Measure malformed-patch rates and task completion on the same model before attributing a score improvement.

## Further opportunities

### Reuse transport state and reduce time before tools start

Roder creates a new `reqwest::Client` for every request at `provider.rs:898` / `1039`, including each internal search continuation. It always sends the full mapped history. The core collects all tool calls at `runtime.rs:4559` and invokes them only after the entire response has ended at `4880`.

Codex dispatches work at `OutputItemDone`, retains in-flight tool futures, and drains them before follow-up sampling (`session/turn.rs:2650`, `2753`, `3137`). Its turn-scoped client reuses WebSocket state, preconnects/prewarms, and uses `previous_response_id` only when request properties and history prefixes match (`client.rs:1394`, `1439`, `1995`). September 23–24 changes improve overlap and idle connection repair.

Start with a pooled HTTP client owned by the engine, stable thread cache keys, and measurements of request setup latency, time to first tool start, serialized bytes, and cached-token usage. Then consider eager dispatch on completed output items and WebSocket deltas. Eager dispatch must preserve Roder's batch-wide `agent_swarm` exclusivity rule, tool authorization, execution ordering, interruption cleanup, and result deduplication. No latency/cost gain is claimed without measurement.

### Strengthen compaction and request-size accounting

Automatic OpenAI compaction, durable opaque boundaries, and two bounded context-limit recovery attempts already exist. Manual and emergency compaction still use ordinary LLM/deterministic text summaries (`compaction_runtime.rs:103`, `transcript.rs:200`). `force_compact_thread` selects the runtime default provider/model at `compaction_runtime.rs:53` rather than resolving the target thread's selection.

A next slice should resolve the thread selection correctly, use a provider compaction capability where supported, and verify that opaque state, user goals, pending call/output pairs, and recent tool evidence survive resume. Codex's current remote V2 compaction uses a specialized streamed request and shared retry handling (`compact_remote_v2.rs:386`); its history reconstruction also retains the relevant instruction context. This is an architectural opportunity, not a claim that Roder lacks all remote compaction or that Codex V2 is a drop-in `/responses/compact` implementation.

Roder's local token estimate excludes provider metadata (`compaction.rs:65`) and does not count tool schemas or image data. Individual textual tool outputs are already capped/spilled (`tool_output.rs:1`, `tool_execution.rs:686`), but no final serialized Responses byte-budget check exists. A 10 MiB image encoded as base64 can dominate the outgoing body despite a small text estimate. Codex's September 24 optional-metadata budget uses serialized message size and sheds optional observations without rewriting canonical history (`client_tool_metadata.rs:10`); it is not a general guarantee that every request fits 15 MiB. Add byte and image accounting appropriate to each actual backend limit.

## Recommended implementation order

1. **Parser and terminal invariants:** UTF-8, finish at terminal SSE events, require a canonical inference outcome. Extract the production SSE state machine from the 5,045-line provider file as part of this work.
2. **Tool-search contract:** current calls/full-schema outputs, durable replay, explicit continuation exhaustion.
3. **Unified recovery:** typed errors, interactive retries, Retry-After deadlines, one terminal failure/Stop hook. Extract the sampling/recovery portion from the 10,230-line runtime file.
4. **Steering and item persistence:** commit completed items, sampling preemption, replay/cleanup guarantees, wire-level app-server and ACP tests.
5. **Current model tool parity:** custom patch capability/grammar and streamed patch input; narrow same-model A/B checks.
6. **Measured performance and long-session work:** pooled transport, cache keys, eager dispatch where legal, WebSocket continuation, compaction and byte budgets.

Each production crate change requires the normal package changeset. Public runtime/event changes need app-server/ACP coverage. Benchmark comparisons should keep model, reasoning, task windows, instructions, and tool availability fixed.

## Verification and reproducible probes

Existing suites on the real checkout:

```sh
mise exec -- cargo test -p roder-ext-openai-responses --lib --locked
# 83 passed
mise exec -- cargo test -p roder-core --test agent_loop --locked
# 15 passed
```

The checked-in audit harness copies the provider source to a temporary crate, uses the real `roder-core` as a path dependency, copies the repository lockfile, and runs local HTTP/fake-engine fixtures. It changes no production file and calls no model endpoint:

```sh
mise exec -- python3 docs/audits/responses-loop-fixtures/run.py
```

On this snapshot the command intentionally exits nonzero because all eight desired contract assertions fail:

| Probe | Observed behavior |
| --- | --- |
| `audit_split_utf8_preserves_text` | `caf��` instead of `café` |
| `audit_completed_ends_without_waiting_for_eof` | Stream remains open after `Completed` |
| `audit_codex_client_search_shape_is_recognized` | Codex `execution`/`arguments` shape is not recognized |
| `audit_search_limit_returns_error_instead_of_silent_eof` | Five requests; no completion or failure |
| `audit_http_retry_honors_retry_after` | One-second advice ignored |
| `audit_interactive_stream_disconnect_retries` | One invocation instead of recovery |
| `audit_stream_failure_emits_one_terminal_failure` | Two failure events instead of one |
| `audit_steer_preempts_unfinished_response` | Steering remains queued behind blocked sampling |

Provider source SHA256 before adding the isolated test module: `612d56180ead287d98b0cb1a757e6c60faa349d0d5bf7d0e1e067cadc5f66187`. The harness is preserved as audit evidence, not added to the product's normal passing test suite. Short timing windows are used only to detect blocked progress; this is not a performance benchmark.
