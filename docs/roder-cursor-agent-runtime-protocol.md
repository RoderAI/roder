# Cursor AgentService bidi agent-runtime protocol

Reverse-engineered from an authoritative `cursor-agent` v2026.05.28 capture
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
| 5 `exec_client_control_message` | exec flow-control ack (`{5:{1:<empty>}}`) |
| 3 `kv_client_message` | KV keepalive/ack (tiny, `{3:{3:<empty>}}`) |
| 6 `interaction_response` | replies to interactive tools (ask_question, web_search…) |
| 7 `client_heartbeat` | keepalive |

| `AgentServerMessage` field | meaning |
|---|---|
| 1 `interaction_update` | model text / thinking / tool-call display / usage / turn end |
| 2 `exec_server_message` | exec channel: read/write/shell **requests** to run on the client |
| 5 `exec_server_control_message` | exec flow control |
| 3 `conversation_checkpoint_update` | checkpoint |
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
  f7  READ   { f1: path, f2: tool_call_id }
  f3  WRITE  { f1: path, f2: new_content(bytes), f3: tool_call_id }
  f14 SHELL  { f1: command, f2: cwd, f3: timeout_ms, f4: tool_call_id,
               f5: simple_commands[] (repeated), f8: parse_tree, f10, f13,
               f15: description }
  f5  SEARCH (unified ripgrep) { f1: pattern (grep), f2: path, f3: glob (glob),
               f4: output_mode ("files_with_matches" | "content"),
               f14: tool_call_id }
  f4  DELETE { f1: path, f2: tool_call_id }   (confirmed from a live
               composer-2.5 capture; Roder routes it through the policy-gated
               shell tool and mirrors an empty result)
  f10 INIT   { f2: conversation_id }     (handshake)
f15 = message uuid
f19 = routing { f1: session_key, f2: per-msg_key, f3: 0 }
```
Tool-call ids are Anthropic-style `toolu_…`.

## Exec result — `exec_client_message` (`AgentClientMessage` field 2)

```
f1 = seq            (echoes the server's seq)
oneof result (mirrors request field number):
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
        exit:   { f3: { f2: cwd (non-empty), f6: duration_ms } }
  f10 INIT          workspace context push (see Flow step 2)
```

cursor-agent also sets a varint field 39 on every `ExecClientMessage`
(monotonically increasing per exec; semantics unknown). Roder does not send it
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
  (`content`); the client walks the workspace and returns the result structures
  above. Verified live (glob + grep return correct counts, no loop).
- **Unknown exec variants must still be answered.** The `ExecServerMessage`
  oneof has more request slots than read/write/shell/search, and the server
  blocks the turn until a result with the mirrored seq + field number arrives.
  Silently dropping an unrecognized exec frame left the stream stalled until
  the no-progress cap killed the turn ("stuck on tool calls", observed with
  composer-2.5). The client now decodes any unhandled slot as an Unknown exec,
  replies with a mirrored empty result (`{1: seq, <field>: {}}`) plus the usual
  `exec_client_control_message` ack, and surfaces the call in Roder's timeline
  as `cursor_unsupported_tool` so the request is visible (and capturable via
  `RODER_CURSOR_CAPTURE_FRAMES`) instead of hanging.
- **Exec results count as progress.** Servicing an exec (including a slow tool
  run or a user approval wait) must reset the no-progress clock; otherwise a
  tool that takes longer than the cap ends the turn right after its result is
  sent, before the model can continue.
- **Heartbeats.** Send `client_heartbeat` (`AgentClientMessage{7:<empty>}`)
  periodically; the server resets long turns without them.
- **No overall request timeout.** A bidi turn can run for minutes; the client
  must not impose a short `reqwest` request timeout (a 62.5s one previously
  manifested as `error decoding response body` on any turn the model spent more
  than ~1 minute on — affecting long edits/searches, not just shell). Stalls are
  bounded instead by a per-read idle timeout and a "no meaningful progress" cap.
- **SHELL** is streamed as start + stdout (`14:{1:{1:stdout}}`) + exit
  (`14:{3:{2:cwd,6:duration_ms}}`), followed by the seq-echoing exec_control
  ack. The cwd in the exit message must be non-empty (fall back to the
  workspace when the request carries no cwd). Resolved: the previous
  "shell-terminal turns emit no final answer" nuance was the empty exec_control
  ack — with the seq echoed, the model resumes immediately after the shell
  result and closes the turn with its final message (verified live with
  composer-2.5: shell-terminated turn completes in seconds with a text answer).

## Implications for Roder

The agent loop is **same-stream**: the client keeps the Run request body open and
writes exec results back into it; the server resumes the same response stream.
Roder's normal per-turn inference provider cannot model this. A dedicated bidi
client must:
- keep the Run stream open (channel-fed request body),
- decode `exec_server_message` read/write/shell requests,
- execute them through Roder's tool registry + policy (provider→runtime tool
  callback) and reply with `exec_client_message` results,
- emit `interaction_update` text as Roder `MessageDelta`s,
- send control/heartbeat frames to keep the session alive,
- finish when the terminal interaction_update arrives.
