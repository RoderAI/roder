# Cursor AgentService bidi agent-runtime protocol

Updated against Cursor Desktop 3.25.9 on 2026-10-21 (`extensions/cursor-agent-host/dist/main.js`, generated protobuf descriptors), and reverse-engineered from an authoritative `cursor-agent` v2026.05.28 capture
(read + edit + shell session) via a Node `http2` interceptor. This documents the
`agent.v1.AgentService/Run` **bidirectional streaming** RPC that drives Cursor's
agentic tool loop, which a dedicated Roder client must speak to complete
end-to-end edits (the simple inference provider cannot — it breaks the turn at
the first tool call).

Endpoint: `POST https://agentn.global.api5.cursor.sh/agent.v1.AgentService/Run`
(HTTP/2, Connect framing: `1 flag byte + 4-byte BE length + protobuf payload`).

## Channels

The request and response are each a stream of Connect frames. Each frame's
payload is an `AgentClientMessage` (client→server) or `AgentServerMessage`
(server→client), a oneof:

| `AgentClientMessage` field | meaning |
|---|---|
| 1 `run_request` | initial `AgentRunRequest` (prompt, model, mode) |
| 2 `exec_client_message` | exec channel: INIT context push + tool **results** |
| 5 `exec_client_control_message` | exec flow-control stream close (`{5:{1:{1:seq}}}`) |
| 3 `kv_client_message` | KV PUT ack (`{3:{1:seq,3:<empty>}}`) |
| 6 `interaction_response` | replies to interactive tools (ask_question, web_search…) |
| 7 `client_heartbeat` | keepalive |

| `AgentServerMessage` field | meaning |
|---|---|
| 1 `interaction_update` | model text / thinking / tool-call display / usage / turn end |
| 2 `exec_server_message` | exec channel: read/write/shell **requests** to run on the client |
| 5 `exec_server_control_message` | exec flow control |
| 3 `conversation_checkpoint_update` | opaque `ConversationStateStructure` with blob references |
| 4 `kv_server_message` | KV channel |

## Flow (observed)

1. Client → `run_request`.
2. Client → `exec_client_message{10:…}` INIT: workspace context push (skill
   files, repo context) as `f10 → f1 → f1 → repeated f2{1:path, 2:content, 3:…}`.
3. Client → `exec_client_control_message`.
4. Loop: server → `exec_server_message` (read/write/shell); client executes
   locally and → `exec_client_message` result, plus periodic
   `exec_client_control_message`, `kv_client_message`, `client_heartbeat`.
5. Server streams `interaction_update`s with the model's text; turn ends via a
   terminal interaction_update.

## Exec request — `exec_server_message` (`AgentServerMessage` field 2)

```
f1  = seq           (varint, monotonic)
oneof request:
  f7/f29 READ { f1:path, f2:tool_call_id, f4:offset?, f5:limit? }
  f3  WRITE  { f1: path, f2: new_content(bytes), f3: tool_call_id }
  f14 SHELL  { f1: command, f2: cwd, f3: timeout_ms, f4: tool_call_id,
               f5: simple_commands[] (repeated), f8: parse_tree, f10, f13,
               f15: description }
  f5  SEARCH (unified ripgrep) { f1: pattern (grep), f2: path, f3: glob (glob),
               f4: output_mode ("files_with_matches" | "content"),
               f14: tool_call_id }
  f4  DELETE { f1: path, f2: tool_call_id }   (confirmed from a live
               composer-2.5 capture; Roder routes it through the policy-gated
               shell tool and returns typed success/error)
  f10 INIT   { f2: conversation_id }     (handshake)
  f8  LS     { f1:path, f2:ignore[], f3:tool_call_id }
  f9  DIAGNOSTICS { f1:path, f2:tool_call_id }
  f11 MCP    { f1:name, f2:map<string,Value> args, f3:tool_call_id,
               f4:provider_identifier, f5:tool_name }
f15 = message uuid
f19 = span_context
f55 = accept_hook_additional_contexts (bool metadata)
f57 = machine_id (string metadata)
```
Tool-call ids are Anthropic-style `toolu_…`.

## Exec result — `exec_client_message` (`AgentClientMessage` field 2)

```
f1 = seq            (echoes the server's seq)
oneof result (the supported native slots below match the request number):
  f7  READ result   { f1: { f1: path, f2: content(bytes), f3: total_lines, f4: file_size } }
  f3  WRITE result  { f1: { f1: path, f2: lines_added, f3: byte_size } }
  f5  SEARCH result
        glob:  { 1:{ 2:path, 3:"files_with_matches", 4:{ 1:root, 2:{ 2:{ 1:relpath*, 2:count } } } } }
        grep:  { 1:{ 1:pattern, 2:path, 3:"content", 4:{ 1:root,
                 2:{ 3:{ 1:{ 1:relpath, 2:{ 1:line, 2:text } }*, 2:count, 3:count } } } } }
  f14 SHELL result  streamed across multiple messages with the same seq
        (shapes re-verified against a cursor-agent v2026.06.12 capture):
        start:  { f4: { f1: { f1: 1 } } }
        stdout: { f1: { f1: <chunk> } }
        exit:   { f3: { f1:exit_code, f2:cwd (non-empty), f6:duration_ms } }
  f10 INIT          workspace context push (see Flow step 2)
```

cursor-agent also sets a varint field 39 on every `ExecClientMessage`
(monotonically increasing per exec; `local_execution_time_ms` in the current schema). Roder does not send it
and the server accepts results without it.

**exec_client_control_message must echo the serviced seq**:
`AgentClientMessage{ 5:{ 1:{ 1:seq } } }` (capture hex `0a020801` after exec
seq 1). An ack without the seq (`{5:{1:<empty>}}`) is accepted for single-frame
results (read/write/search) but after a *streamed* shell result the server
never learns the exec finished — the model never resumes and the turn sits
"stuck on the shell tool" until the no-progress cap ends it. This was the root
cause of shell-terminated turns producing no final answer.

## `run_request` (`AgentRunRequest`)

Same shape Roder already encodes:
```
f1 conversation_state (empty)
f2 ConversationAction { f1 user_message_action { f1 UserMessage {
     f1 text, f2 message_id, f4 mode = AGENT_MODE_AGENT(1) } } }
f5 conversation_id
f9 requested_model { f1 model_id, f3 repeated {f1 key, f2 value} }
   params seen: thinking=true, context=300k, effort=high, fast=false
f12 = 0
f16 = conversation_id
```
Note: cursor-agent does **not** put workspace context in `run_request`; context
arrives via the exec INIT message (step 2).

## Implementation notes (roder-ext-cursor `bidi.rs`)

Verified end-to-end: `cursor/claude-opus-4-8` completes read→write edits through
Roder, with each tool routed through Roder's registry + policy (shown as
`Read File` / `Write File` in the TUI). Required pieces discovered while
building the client:

- **Model params.** `requested_model.f3` must include `effort=high` (plus
  `thinking=true`, `context=300k`, `fast=false`). Without `effort=high` the
  model does minimal work and stops after one read.
- **kv acks.** The server PUTs conversation state on the `kv_server` channel
  (`AgentServerMessage` field 4, `{1:seq, 3:{put}}`). Each must be acked with
  `AgentClientMessage{ 3:{ 1:seq, 3:<empty> } }`. Without acks the server cannot
  persist the turn and ends it after the first tool call. **This is the key
  unlock for multi-step turns.**
- **exec INIT.** Send `AgentClientMessage{ 2:{ 10:{1:{1:{2:[files]}}} } }` after
  the run request (empty file list is accepted).
- **exec control.** Ack each exec result with
  `AgentClientMessage{ 5:{ 1:{ 1:seq } } }` — the ack must echo the exec seq
  (see above; an empty ack stalls shell results).
- **READ** results carry the raw file bytes (not Roder's line-numbered output).
- **SEARCH** (exec field 5) covers both glob (`files_with_matches`) and grep
  (`content`); the client uses Roder's approved grep/glob results and returns the structures
  above. Relative paths stay relative to the runtime workspace. Verified live (glob + grep return correct counts, no loop).
- **Unknown exec variants must fail explicitly.** Use `AgentClientMessage{5:{2:{1:id,2:error,4:error_code}}}` (`ExecClientThrow`). An empty result in the request's field number is invalid for newer Pi calls: request fields 45–51 correspond to result fields 46–52. Roder sends a failure and the usual stream-close id rather than pretending the operation succeeded.
- **Typed failures.** Read errors use result field 2; write errors field 5; delete errors field 7; grep/ls/diagnostics/MCP errors field 2. Shell execution failures retain the actual nonzero exit code, distinct from approval rejection.
- **Structured executor results.** File content and metadata, search matches and directory entries come from `TurnToolOutcome.data`. Rendered/line-numbered tool text is for the transcript. No local filesystem or shell fallback runs inside the provider.
- **Completion and usage.** Only a terminal turn update yields successful completion. EOF, an early Connect end-stream, and inactivity are errors. The heartbeat task is aborted on stream cancellation as well as normal exit. `TurnEndedUpdate` fields 1–5 are input, output, cache-read, cache-write, and reasoning tokens; these are retained in metadata and input/output counts are preferred over estimates.

## Current coverage

The implemented exec subset is read (7/29), write (3), delete (4), unary/streamed shell (2/14), grep/glob (5), directory listing (8), context (10), MCP invocation (11), and MCP state (36). Hosted discovery and `update_plan` dispatch were verified live with `composer-2.5`. Diagnostics (9), multiline grep, and other exec variants return explicit unsupported errors. This is a partial schema, not a claim of support for Cursor's entire agent host.

## Checkpoints and KV state

`AgentServerMessage.f3` carries `ConversationStateStructure`, not the similarly named inline `ConversationState`. Its root prompt messages (f1) and turns (f8) are repeated byte ids referring to the KV store. An empty state plus a stable conversation id loses context on a subsequent turn.

KV requests use `KvServerMessage{1:id,2:GetBlobArgs{1:blob_id}}` or `{1:id,3:SetBlobArgs{1:blob_id,2:blob_data}}`. PUT replies are `{3:{1:id,3:{}}}`. GET replies are `{3:{1:id,2:{1:blob_data}}}`; a missing blob returns `GetBlobResult.error` (f2), containing `Error{1:message}`. A GET must never receive an empty PUT acknowledgement.

Roder persists the checkpoint and blobs as opaque base64 in provider metadata. The next run sends the checkpoint as `AgentRunRequest.f1` and supplies blobs as repeated `f17 PreFetchedBlob{1:id,2:value}`. The store is limited to 64 MiB; overflow is an explicit error. Compacted Roder context starts a fresh Cursor state and imports the summary in the prompt. Live tests exercise checkpoint restoration and imported text history; TUI tests exercise file/tool results across turns.
