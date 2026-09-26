# Responses loop implementation and verification

Reference: local Codex `8b78f4796605bda8e31329537f5fef036e405e66`.
Frozen Roder baseline: `816974be81b5cb7a1d249c14bc5bd26d7afb14ff`.
Branch: `pz/responses-loop-parity`.

This records the follow-up to [the original audit](2026-09-26-responses-agent-loop.md). The original audit is a historical snapshot. Implementation and verification are local; no remote refresh, submission or deployment was performed.

## Audit closure

| Finding or opportunity | Implementation and regression evidence |
| --- | --- |
| 1. Split UTF-8 SSE | Byte buffering, strict decoding, LF/CRLF/bare-CR framing, linear scanning and a 16 MiB frame limit. Split Unicode and malformed bytes are covered. |
| 2. Wait for EOF after completion | HTTP and core stop at canonical completion; silent EOF is a typed interruption. A held-open HTTP fixture measures this change. |
| 3. Stale client search contract | Canonical client execution/query/limit/call ID, complete loaded schemas and paired search replay on HTTP and WebSocket. |
| 4. Silent search exhaustion | Three bounded rounds; fail before another request. Wire tests count requests and terminal events. |
| 5. Interactive recovery | Typed transient failures across profiles, a shared per-sampling retry budget, quota/auth/protocol classification and preserved request/response IDs. |
| 6. Server retry deadlines | Delta seconds and HTTP dates become monotonic retry advice; wait for the later of local backoff and server advice. Steering interrupts the wait. |
| 7. Duplicate terminal failure | One outer failure and Stop path; cleanup acknowledgement before retry/preemption. Mock tool responses now emit canonical completion. |
| 8. Steering during sampling | Watch preemption of setup, stream reads and backoff; atomic mailbox drain and durable acknowledgement. Immediate cancellation cannot lose the watch receiver or spin on a closed channel. |
| 9. Patch channel/model coverage | Codex grammar custom tools for supported GPT-5/GPT-6 OpenAI and Codex models, including Luna. Raw streaming input and item completion are separate. Older model fixtures retain JSON capability coverage. |
| Transport and tool overlap | Pooled HTTP clients, task cache keys, credential/account-scoped WebSocket sessions, exact-prefix delta continuation, full-history repair and HTTP fallback. Eager reads run through normal routing and are owned by the turn. |
| Compaction and request accounting | Target-task native compaction, preserved instructions/tools and full opaque windows, shared retry/steering, task admission during manual compaction, 15 MiB serialized request guard and conservative image/opaque accounting. |

## Complete apply_patch alignment

The Lark grammar and derived parser, matching and streaming components carry Apache-2.0 attribution. Tests cover anchors, EOF, exact/trailing/trim/Unicode matching, implicit first chunks, blank context, heredoc input, malformed markers, line-numbered errors, add/update/delete/move, overwrite behavior and newline handling. Local and runner execution share matching logic.

The tool accepts the canonical `patch` argument and Codex patch syntax. Source paths and contexts are preverified; duplicate normalized source paths are rejected before writing. Updates are re-read during execution so earlier moves are respected. Default text normalization matches Codex. Edit-core also exposes explicit preservation of existing line endings.

Patch progress is proposed state. Hunk records and diffs use actual before/after filesystem state, including partial failures. Exact rollback is only advertised when the original state can be represented accurately. Move rollback restores an overwritten destination. Symlink deletion unlinks the path rather than its target.

Roder retains workspace authorization. Single-environment invocation parses but rejects environment IDs, matching the selected Codex invocation mode; multiple-environment routing is outside this runtime. Authorization errors and native/ACP event envelopes remain Roder contracts. Runner execution uses its runner filesystem API rather than Codex's local host adapter.

## Integration fixes uncovered during verification

- Completed raw messages, reasoning, calls/search exchanges and opaque boundaries survive retry, steering and disconnection. Replayed completed call IDs do not execute twice; conflicting arguments fail explicitly.
- Native compaction ignores an in-progress boundary and commits only a completed one. A new turn can compact again; the same turn waits for context regrowth.
- Manual compaction holds admission for its target task until the boundary is committed. Other tasks can start, and queued input samples the committed boundary.
- Eager reads accept auto-approved policy decisions. A live Luna rerun exposed a lock deadlock when read futures were polled only between stream events. Turn-owned tasks now continue while item persistence awaits storage; dropping ownership aborts unfinished reads. The concurrent-read regression and final live tasks pass.
- Frozen baseline builds use an isolated Cargo target. This prevents baseline artifacts from contaminating current workspace verification.

## Same-model evaluation

New Harbor, OpenAI tool-search and comparison defaults use **GPT-6-luna**. Explicit older-model experiments and historical results retain their original settings.

The final HTTP comparison holds the model, low reasoning, instructions, tool allowlist and filesystem assertions constant. Both builds passed both tasks with zero patch errors:

| Task | Frozen baseline | Current | Current tools |
| --- | ---: | ---: | ---: |
| Unicode update | 12,195 ms | 8,198 ms | 3 |
| Move, overwrite, add and delete | 12,850 ms | 13,544 ms | 8 |

This small, sequential, unrandomized sample shows successful editing and mixed latency. It does not establish a general performance or quality improvement. First-event timing includes diagnostic metadata and is not time to first token. First tool start, request byte/image metadata and usage are retained in the result artifacts.

WebSocket/native compaction smoke verifies an opaque boundary and a subsequent answer retaining BLUE. The small context estimate can increase; this is continuity evidence, not a token-savings claim. The offline fixture holds terminal SSE open for 150 ms to isolate completion-to-EOF latency; it is not model throughput. Reproduction instructions and result files are in [responses-loop-eval](responses-loop-eval/README.md).

## Verification record

Machine-readable counts and source fingerprint: [verification JSON](responses-loop-verification.json).

- Workspace execution reached all 364 test targets: 3,157 passed, 55 ignored and three failures. The untouched `roder-commands::expand_blocks_shell_includes_by_default_and_in_plan_mode` expects a read-only shell include to fail in Plan mode, while the implementation permits it. The same failure was reproduced in an isolated build of the frozen baseline; its source and test are unchanged. The other two failures (`default_roder_home_dir_uses_home_roder` and `tui_config_path_targets_home_roder_config`) came from the child process's temporary `RODER_DATA_DIR`; both passed with that override removed.
- Core: 232 unit tests plus integration suites passed, including retry/history/preemption, concurrent eager reads and compaction admission. Responses provider: 113 passed. Edit-core: 36 unit tests, one fixture matrix, seven fuzzy tests and six post-edit tests passed. Coding tools: 126 passed.
- Promoted audit harness: eight provider probes and three runtime regressions passed.
- `cargo clippy --workspace --all-targets --locked` passed with existing workspace warnings. Release config generation and whitespace checks passed.
- Harbor adapter Python suite: 496 passed with the new Luna defaults.
- Real Rust CLI ACP stdio smoke passed initialization at protocol 1, session creation and a mock prompt ending with `end_turn`.
- Luna HTTP: 2/2 on current and 2/2 on frozen baseline. Current WebSocket: 2/2; native compaction continuity: 1/1. No patch errors in these live fixtures. Held-open SSE: 20/20 each, median 200.833 ms baseline and 40.705 ms current.
- Optional public transport checks: ACP 4/4, patch contracts 2/2, steering 2/2 and app-server E2E 126 passed, one ignored. Three failures also reproduced on the frozen baseline were filtered for this final run: `providers_clear_removes_api_key` (saved Cursor login state), `runners_methods_list_select_status_and_delete_destination` (runner token expectation) and `tools_list_discovers_configured_web_search_without_secret_material` (an authentication label in the payload). The initial current and baseline unfiltered runs retain those same failures; the final filtered pass does not imply a clean unfiltered suite.
- The changeset names all eight modified released packages. Its committed branch coverage is checked against the frozen baseline. A full Terminal-Bench quality run, source landing and deployment remain separate activities from this audit goal.

## Release follow-up

The two config/authentication-dependent unit failures are fixed: registry configuration tests run in isolated child processes and provider selection tests supply explicit authentication state. The shell-include integration expectation now matches the current Plan process policy. A fresh unfiltered `cargo test --workspace --locked --no-fail-fast` run passed: 3,163 test executions, zero failures and 55 ignored.

The release changeset includes the reverse dependency closure of the shared API changes. Knope now updates workspace dependency requirements alongside crate versions, preventing registry builds from selecting old API implementations. The verified release preview targets Roder 0.2.0 and its dependent crates. Registry README and publish-order gates pass.
