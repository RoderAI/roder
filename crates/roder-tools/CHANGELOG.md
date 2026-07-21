## 0.1.5 (2026-07-21)

### Fixes

#### Keep remote exec timeout semantics honest

Normalize remote one-shot commands to a finite ten-minute maximum before the
provider request is built, so the local outer timeout, runner lease, and tool
result metadata all report and enforce the same bound during multi-hour turns.

## 0.1.4 (2026-07-21)

### Features

#### Lazily initialize per-thread remote runners and support one-shot exec

Runner-bound threads now create or resume their remote session only when an
approved native workspace tool first executes. Concurrent first tool calls
share one initialization, the live session is reused across later tools and
turns, and its state is persisted before the first tool runs so a new process
can rejoin it. Text-only and host-executed MCP turns do not wake a runner, and
failed initialization remains retryable without falling back to local tools.

`exec_command` now runs non-interactive one-shot commands through a remote
runner with remote working-directory scoping, shell/login handling, deadlines,
timeouts, output truncation, and the existing Codex-shaped result payload.
Remote TTY and stdin-continuation requests fail clearly instead of executing
on the host. Hosted runtimes can also disable local workspaces completely, so
a missing or malformed runner binding fails closed before any native workspace
tool can touch the host filesystem.

### Fixes

#### Bound and cancel detached remote commands

Remote command requests can now carry a wall-clock process lease. Remote shell
and exec tools request provider cancellation when they time out or are dropped
by turn interruption instead of allowing detached work to continue.

The Blaxel runner starts every command as a uniquely named process with a
finite server-side keep-alive timeout, polls the process API for commands that
run beyond the synchronous 60-second window, advertises cancellation, and
force-kills the process group when Roder cancels the command.

## 0.1.3 (2026-07-21)

### Features

#### `unified_exec` tool for Codex tool-shape parity

Adds `unified_exec`, a single-tool wrapper over the same `ExecSessionManager`
that already backs `exec_command`/`write_stdin`, matching Codex's persistent
PTY tool shape that gpt-5.5 was RL-trained on: `{ input, session_id?,
timeout_ms? }`. Omitting `session_id` starts a new session running `input` as
a shell command; passing one writes `input` to that session's stdin. Both
cases return output collected up to `timeout_ms` (default 1000ms) plus the
session ID if the command is still running — `timeout_ms` bounds the wait for
output, it does not kill the process. `session_id` is accepted and returned as
a string, matching Codex's wire shape, and a plain integer is still accepted
on input. `exec_command` and `write_stdin` stay registered and unchanged; all
three tools share the same session pool, so a session started by one can be
driven by another.

#### Path-based `view_image` tool for vision tasks

Adds a native `view_image(path)` tool that mirrors Codex's semantics: it reads
an image file (png/jpeg/gif/webp, validated by magic bytes, capped at 10 MiB),
base64-encodes it, and returns it as an image content block in the tool result
so the model sees the pixels. It reads through the workspace backend, so it
works against both local and remote-runner workspaces.

- `roder-tools`: new `view_image` tool (registered alongside the builtin coding
  tools); a `read_bytes` method on the workspace backend for binary reads; and
  `media_attach` now degrades to actionable guidance (pointing at `view_image`)
  instead of hard-failing when called without raw base64 bytes, so it no longer
  burns the consecutive-tool-failure budget in headless/eval runs.
- `roder-api`: `VIEW_IMAGE_DISPLAY_KEY`, a reserved `display_payload` key that
  carries the image block from tool result to provider.
- `roder-ext-openai-responses`: `function_call_output` now forwards a
  `view_image` result as an `input_image` content block (when the model
  supports images), falling back to the plain string output otherwise.

### Fixes

#### Freeform apply_patch on the Responses custom-tool channel

Advertise `apply_patch` on the OpenAI Responses freeform/custom tool channel
(`type:"custom"`) for the gpt-5.5 family, matching the channel the model was
RL-trained to emit patches on. `ToolSpec` gains a `freeform_input_field` marker
(default `None`, so ordinary function tools are unchanged); the Responses
provider serializes marked tools as `type:"custom"`, parses `custom_tool_call`
outputs into the normal tool-dispatch path, and replays their results as
`custom_tool_call_output`. Non-gpt-5.5 models and every other provider keep the
JSON `type:"function"` shape. The `apply_patch` handler accepts both the JSON
`{ "patch": ... }` arguments and the raw freeform body.

#### Allow create_goal after a completed goal

`create_goal` only fails while an active goal is in progress. Completed, blocked, paused, or limited goals can be replaced so resumed sessions can start the next objective.

## 0.1.2 (2026-06-26)

### Fixes

#### Allow glob to search absolute paths outside the workspace

Match `glob` behavior with the other file search tools by allowing absolute patterns to search directly when filesystem access is unrestricted, while preserving workspace-only scoping when configured.

## 0.1.1 (2026-06-15)

### Fixes

#### First-party image generation providers (OpenAI GPT Image and Google Gemini Nano Banana)

Provider-neutral image generation through the core media API: an image-capable
`MediaGenerationRequest`/multi-output `MediaGenerationResponse` contract, a new
`ProvidedService::MediaGenerator` extension service, a runtime media generation
service backing the canonical `media_generate_image` tool with a deterministic
offline fallback, new `roder-ext-openai-images` (`gpt-image-2` plus legacy ids)
and `roder-ext-google-images` (Nano Banana 2/Pro/base) provider crates,
`[media.image_generation]` config, `media/image/providers/list` and
`media/image/generate` app-server methods, `roder media` CLI commands, palette
entries, and regenerated schemas/SDK stubs. Live provider smokes stay opt-in
behind `RODER_OPENAI_IMAGE_LIVE` / `RODER_GEMINI_IMAGE_LIVE`.

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.

#### Make grep and glob tools reliable for agents

Transcript analysis showed 44% of grep calls and 46% of glob calls returned empty results, mostly from the tool contract fighting model conventions. This change:

- treats grep queries as regex by default (models habitually send `a|b` patterns without setting `regex: true`, which previously matched nothing as a literal), with a clear error and literal-mode hint for invalid patterns
- replaces empty zero-match output with an explanation of what was searched (mode, case, scope, engine, file counts) plus remedial hints
- backs glob with globset, adding `{a,b}`, `[..]` and proper `**` support, and resolves absolute or `~` patterns inside the workspace instead of silently matching nothing; patterns outside the workspace now error clearly
- searches explicitly scoped ignored directories (node_modules, dist, gitignored paths) by relaxing ancestor ignore rules when the caller names such a path directly
- passes the canonicalized search path to the engine so `~`, symlink, and case differences no longer silently drop every indexed match
- invalidates the cached search index after shell and exec commands so files they create are found by the next grep
