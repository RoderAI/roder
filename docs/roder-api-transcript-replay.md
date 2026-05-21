# Roder API Transcript Replay

Roder API transcripts are JSONL files that capture the public app-server boundary used by the TUI: JSON-RPC requests and responses, app notifications, runtime events, user input, terminal size, and optional normalized frame snapshots.

## Recording

Record a normal TUI session:

```sh
roder --record-api-transcript .roder/transcripts/bug.jsonl
```

Include rendered frame snapshots when the visual state is part of the regression:

```sh
roder --record-api-transcript .roder/transcripts/bug.jsonl --record-ui-frames
```

Recording writes a header first, then monotonic `seq` records. API records are captured through the app-client wrapper, and UI input records are captured before the TUI handles the key, paste, mouse, or resize event.

## Inspecting

Each line is one JSON record:

```sh
sed -n '1,20p' .roder/transcripts/bug.jsonl
cargo run -p roder-cli -- replay .roder/transcripts/bug.jsonl --headless
```

Headless replay is offline and side-effect-free by default. It must not call providers, install plugins, execute tools, or contact remote app-servers.

## Live Replay

Live replay is only for protocol drift debugging and is gated:

```sh
RODER_LIVE_REPLAY=1 roder replay .roder/transcripts/bug.jsonl --live
```

Do not use live replay for normal regression tests. It can send recorded API requests to a local app-server.

## Redaction

Before committing a transcript, inspect it for local paths, bearer tokens, API keys, pasted secrets, command payloads, and provider-specific auth fields. Use the transcript redactor APIs in `roder-api-transcript` for programmatic cleanup, and prefer `<redacted>` placeholders over deleting the record shape.

## Updating Frames

When intentional UI changes alter a fixture, regenerate or edit the relevant `ui.frame` record so:

- `cols` and `rows` match the target terminal size.
- `text` is normalized text with trailing spaces and trailing blank rows removed.
- `textHash` is `sha256:` plus the SHA-256 of the normalized text.

For a quick local hash:

```sh
printf '%s' 'normalized text' | shasum -a 256
```

## Adding A Regression Fixture

1. Record or create a minimal transcript under `tests/fixtures/api-transcripts/`.
2. Keep only the public API, UI input, and frame records needed to prove the flow.
3. Replay it locally:

```sh
cargo run -p roder-cli -- replay tests/fixtures/api-transcripts/startup.jsonl --headless
```

4. Add or update focused tests in `roder-cli` or `roder-tui` when the fixture covers a new behavior.

## Stability Rules

- Use deterministic timestamps such as `1970-01-01T00:00:00Z` for committed fixtures.
- Use `<redacted>` for `cwd` and other local paths.
- Avoid random ids unless the id itself is the behavior under test.
- Keep terminal dimensions explicit and small.
- Do not include terminal escape sequences in frame text.
- Do not include provider output from live services in committed fixtures.

## Optional Tmux Smoke

The live recorder smoke is opt-in:

```sh
RODER_TUI_RECORD_SMOKE=1 scripts/roder-tmux-smoke api-transcript-record-replay
```

The smoke starts the TUI in tmux with transcript recording enabled, exits it, and then runs headless replay against the captured transcript.
