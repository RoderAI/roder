# Agent backends

Roder's default backend runs Roder's own inference and tool loop. A complete
agent backend owns its own conversation, inference, tools, and policy. Backend
extensions register an `AgentBackend` service in the extension registry. Roder
app-server maps its existing thread, turn, event, and approval API to the
selected backend. Clients, including the existing Roder TUI, continue to use
the Roder protocol.

## Switching backends in the TUI

Press Ctrl+P and choose **Agent backend** to switch between Roder, Codex, and
Antigravity. A backend is bound when the app-server starts, so choosing a
different one restarts Roder with `--backend <id>` and keeps your other flags,
such as `--yolo` and `--config-dir`. The restart drops `--model` and `resume`
because models and threads belong to a backend. Antigravity runs its guided
setup on first use. A running turn must finish or be interrupted first.

## Codex

Start a Codex-backed Roder session:

```sh
roder --backend codex
```

Select a Codex model or resume a Codex thread by its id:

```sh
roder --backend codex --model gpt-6-sol
roder --backend codex resume <codex-thread-id>
```

Roder spawns `codex app-server` from `PATH` in the current directory and speaks
the app-server JSONL protocol over its private stdio stream. Codex must be
installed and signed in. The existing Roder UI handles streamed messages,
tool activity, command output, file edits, token usage, and approval dialogs. Roder's policy mode maps to
Codex's approval and sandbox policy: Default uses on-request/workspace-write,
AcceptAll uses never/workspace-write, Plan uses on-request/read-only, and
Bypass uses never/danger-full-access. Codex's own configuration still supplies
other settings.

The same backend works through Roder's app-server:

```sh
roder app-server --backend codex
```

An app-server client continues to call Roder's `thread/start`, `turn/start`,
`turn/steer`, and `turn/interrupt` methods and receives Roder notifications.
Roder's model picker reads Codex's `model/list`; saved threads come from Codex's
`thread/list` and `thread/resume`.

The `roder-ext-codex-backend` crate translates Codex's thread, turn, item, and
approval messages to Roder's backend protocol. Other agent runtimes can
implement the same interface without joining Roder's inference provider or tool
registry. Codex client actions beyond command approvals, file-change approvals,
permission grants, and option-based user questions currently receive an explicit unsupported-method
response, so the turn fails instead of waiting indefinitely for an answer.

## Google Antigravity subscriptions

Antigravity runs as an official ACP agent backend, using the same `agy acp`
integration approach as T3 Code. This is not the Gemini API-key inference
provider: Antigravity owns inference, tools, and its conversation state, while
Roder supplies the UI and app-server protocol. Roder's native tools and inference
extensions do not run inside an Antigravity-backed conversation.

Start Roder with the Antigravity backend:

```sh
roder --backend antigravity
```

If the official CLI is missing, Roder opens a setup wizard before initializing
the chat:

1. **Install:** press Enter to authorize a managed, per-user download from
   Google's official release service. Roder checks the published SHA-512,
   safely extracts the executable, and verifies its version before activation.
   No sudo, shell installer, or system PATH edit is needed.
2. **Sign in:** press Enter to continue through the official Google account
   flow. Roder temporarily restores the normal terminal so `agy acp --login`
   can show its URL and accept the temporary code without Roder intercepting it.
3. **Verify:** Roder checks account access and discovers the available models.
   Press Enter when ready to continue into the chat.

The wizard shows download progress; Escape cancels setup, and `r` rechecks
installation/account status. Downloads start only after consent. Allow at least
1.25 GiB for the runtime and temporary extraction. Errors can be retried without
discarding a working managed installation.

A Roder-managed runtime is preferred when installed; otherwise an existing
`agy` on PATH is reused. Managed files live under Roder's config directory
(`RODER_CONFIG_DIR`, then `RODER_DATA_DIR`, then `~/.roder`). The same runtime is
used for chat and these explicit account commands:

```sh
roder auth login antigravity
roder auth status antigravity
roder auth logout antigravity
```

Piped/noninteractive startup and app-server clients never initiate an install or
interactive login; run the TUI setup first. An already-authenticated runtime
skips the wizard.

To run only setup without opening a conversation, use `roder setup antigravity`.

The login command delegates to `agy acp --login`, keeping credentials in the
official CLI's credential store. Like T3 Code, Roder selects the CLI's
SSH-style terminal sign-in and file-backed credential mode consistently for
both login and agent sessions. It does not read T3 Code's private profiles,
extract Google tokens, or treat a Gemini API key as a subscription. An IDE,
T3 Code, or standalone local-keyring login can use a different credential store;
use `roder auth login antigravity` for this integration rather than assuming
those logins are shared. Ambient Google/Gemini API credentials are removed from
the child process so they cannot silently switch the personal-subscription flow
to API billing.

Models are discovered from the signed-in Antigravity account, not a hard-coded
Gemini catalog. Choose a model in Roder's model picker, or pass its exact
account-provided ID:

```sh
roder --backend antigravity --model '<model-id>'
roder app-server --backend antigravity
```

For full access without native Antigravity permission prompts:

```sh
roder --backend antigravity --yolo
```

When the thread starts in Roder's Bypass mode, its ACP process receives
`--dangerously-skip-permissions`. Default, AcceptAll, and Plan launches do not
receive that flag. A process launched with native permission bypass cannot
later tighten its policy in place: start a new thread/session in the safer mode.
Switching a normally launched session to Bypass uses Roder's host-side approval
handling instead.

App-server clients continue to use Roder's `providers/list`, `model/list`,
`thread/start`, `turn/start`, `turn/interrupt`, and approval methods. The selected
provider ID is `antigravity`. Missing sign-in or subscription access is reported
as an error rather than falling back to another provider or an API key.

Session listing and resume depend on the installed agent's advertised ACP
capabilities. Archiving stops Roder's active Antigravity process; it does not
delete the official CLI's saved conversation, which may still appear in that
agent's session listing.

### Validation

The backend crate includes offline protocol tests using a fake ACP subprocess.
These cover the adapter without Google credentials. Its ignored live test
requires an installed and signed-in `agy`; run it explicitly after login:

```sh
cargo test -p roder-ext-antigravity-backend
RODER_LIVE_ANTIGRAVITY=1 cargo test -p roder-ext-antigravity-backend --test live -- --ignored --nocapture
RODER_LIVE_ANTIGRAVITY=1 cargo test -p roder --test antigravity_backend_process -- --ignored --nocapture
```

The live tests use your subscription allowance. The process-level test runs the
real Roder app-server and installed Antigravity runtime in an isolated temporary
workspace. A passing mocked test does not prove that a Google account has
subscription access.

External agent backends are currently supported by Roder's TUI and app-server,
not `roder exec` or Roder's editor-facing `roder acp` server. Those commands
reject an external `--backend` instead of silently using native inference.
