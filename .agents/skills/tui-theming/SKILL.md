---
name: tui-theming
description: Keep Roder/gode TUI visuals readable, compact, and consistent. Use when changing roder-tui timeline, transcript, composer, tool rows, dialogs, footer/status, phase messages, or terminal colors.
---

# TUI Visuals

## Instructions

- Target `crates/roder-tui` first. Older Go/Bubble Tea paths are historical unless the user explicitly points at them.
- Route new colors through the local `Theme` struct in `crates/roder-tui/src/app.rs` or a nearby extracted theme module. Do not scatter `Color::White`, `Color::Black`, raw grays, or one-off `Rgb` values through render code.
- Primary text should usually use terminal default/reset for contrast. Use semantic roles for muted text, strong text, borders, accent, tool, running tool, error, mode, selection, and status/build rows.
- Keep the app transcript-first. Do not add a side panel unless explicitly requested.
- Avoid decorative cards, boxed panels, and wide gutters in the timeline. The main transcript should read like a clean terminal conversation, not a dashboard.
- Preserve dark-theme intent, then choose a light-theme counterpart with real contrast. Add focused tests when touching colors or roles.

## Timeline Visual Target

The current timeline goal is closer to Grok/OpenCode than the old requested/completed event log:

- User prompts render as full-width transcript blocks with a subtle left state rail or border. The block should have minimal padding and no extra left column before the timeline content.
- Reasoning and phase messages render inline as muted commentary, for example `Thinking:` or `Commentary:`, with the phase label styled separately from the text. Commentary should be visible and preserved, but visually secondary to final assistant output.
- Final assistant text renders as normal transcript text, not inside a card.
- Tool calls render as one row per tool with a compact symbol, title, and key arguments. Show the row immediately when requested, update it in place as it runs/completes, and turn the row red on failure.
- Completed tool output should be expandable through keyboard and mouse interaction. Navigation should work with Tab, arrows, `j`/`k`, and click, while normal typing in the composer keeps working.
- The timeline must scroll independently and keep selected/expanded tool output readable without stealing space from the composer.
- The bottom status/build row should be compact, similar to `Build · Model · elapsed`, with subdued separators and no large box treatment.
- Top running animation should be subtle and fast enough to imply activity without dominating the transcript.

## Composer And Status

- Composer mode should be visually explicit: default, accept_edits, plan, and bypass need distinct colors, and the input border should reflect the active mode.
- Space, Shift+Enter, Cmd+Backspace, Ctrl+W, arrows, and paste should behave like a serious text input. Do not let timeline focus consume composer text keys.
- Queued and steering messages should stay visible but compact. They should not push the main timeline into an unusable height.
- Image attachments should be represented in the composer/transcript by short labels while the provider path carries structured image data; do not paste local file paths into model prompt text as a workaround.

## Verification

- Run `cargo test -p roder-tui` for TUI changes.
- Run `cargo clippy -p roder-tui --all-targets -- -D warnings` before claiming visual or interaction work is done.
- For public app-server or protocol-visible behavior, also use the `maintain-acp-compliance` skill and run the relevant app-server/protocol tests.
- For visual proof, use the `roder-tmux` skill to capture screenshots of the running TUI. Compare against the Grok/OpenCode-style target: compact transcript, visible phase commentary, no side panel, no extra left gutter, scrollable timeline, and expandable tool rows.
