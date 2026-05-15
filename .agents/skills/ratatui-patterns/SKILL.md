---
name: ratatui-patterns
description: Architecture, event-loop, and rendering patterns for ratatui TUIs. Use when building or extending `roder-tui` or any other Rust ratatui application — adding components, popups, async event handling, layout, focus, or message passing.
---

# Ratatui Patterns

Distilled from the official ratatui docs (ratatui.rs) and the most-loved production TUIs in `awesome-ratatui` — `gitui`, `bottom`, `yazi`, `slumber`, `ATAC`, `spotify-player`, `gobang`, `kdash`. Ratatui is an immediate-mode terminal UI library: the whole frame is redrawn from application state on every tick, so architecture work goes into *state*, *events*, and *messages* — not retained widget trees.

## Instructions

### Pick an architecture before writing widgets

Three patterns are blessed by the docs. Pick one and stay with it — mixing styles is what makes ratatui apps hard to maintain.

1. **The Elm Architecture (TEA)** — one `Model`, one `Msg` enum, pure `update(&mut Model, Msg) -> Option<Msg>`, pure `view(&Model, &mut Frame)`. `update` may return a follow-up message so cascading transitions stay explicit. Best for small/medium apps with a single coherent state. Examples: most ratatui tutorial code, smaller awesome-ratatui entries.
2. **Component Architecture** — each widget is a struct implementing a `Component` trait with `handle_events`, `update`, `render`, plus optional `focus`/`hide`/`show`. State lives inside the component. Best for apps with many independent panes. Examples: `gitui` (asymmetric `Component` + `DrawableComponent` traits with `event_pump`/`command_pump` helpers), the official `ratatui/templates/component` template, `gobang`.
3. **Hybrid with an Action enum** — components emit `Action`s (or `InternalEvent`s in `gitui`) into a shared queue; the app loop drains the queue and applies actions to model + components. Decouples "key pressed" (event) from "what changes" (action). The component template uses this; `gitui` calls it `Queue<InternalEvent>` (`Rc<RefCell<VecDeque<_>>>` for single-threaded sharing).

Inside the gode repo, `crates/roder-tui/src/app.rs` is currently a monolithic `TuiApp` mixing event read, draw, and async-server calls. When growing it past one screen, refactor toward **Component + Action queue** — it matches gode's multi-pane direction (transcript / composer / footer / popups) and keeps async server events orthogonal to keyboard input.

### Run the event loop on tokio with `EventStream` + `select!`

For any app using tokio (gode does), do **not** use the blocking `event::poll(Duration::from_millis(50))` pattern. It pegs a thread, fights tokio's scheduler, and makes tick/render rates wobble. Use this canonical async pattern:

```rust
let mut reader = crossterm::event::EventStream::new();
let mut tick = tokio::time::interval(Duration::from_millis(250));   // business logic
let mut render = tokio::time::interval(Duration::from_millis(16));  // ~60fps draw

loop {
    tokio::select! {
        _ = cancel.cancelled() => break,
        Some(Ok(ev)) = reader.next() => tx.send(Event::from(ev))?,
        _ = tick.tick()   => tx.send(Event::Tick)?,
        _ = render.tick() => tx.send(Event::Render)?,
        Some(server_ev) = server_rx.recv() => tx.send(Event::App(server_ev))?,
    }
}
```

Forward to the main loop through an `UnboundedSender<Event>`. The main loop awaits `rx.recv()`, classifies the event, and either updates state or pushes an `Action` onto its queue. Separate `Tick` (state advance — animations, spinners, polling) from `Render` (redraw) so a heavy update can't stall drawing.

Define one canonical `Event` enum: `Init`, `Quit`, `Error`, `Closed`, `Tick`, `Render`, `FocusGained`, `FocusLost`, `Paste(String)`, `Key(KeyEvent)`, `Mouse(MouseEvent)`, `Resize(u16, u16)`, plus app-specific variants (`App(AppEvent)` for server pushes, jobs finishing, etc.).

### Keep `update` / `view` pure-ish and side-effect free

The view function gets a `&Model` (or `&mut self` for components) and a `Frame`, and must do **nothing** but compute widgets. No I/O, no spawning tasks, no allocating long-lived state. The `update`/`handle_action` step is the only place state mutates. Side effects (HTTP, RPC, file I/O) are triggered by *returning a command* — spawn a tokio task that sends a result back as an `Event::App(...)`, never `.await` inside `update`.

This is the bug to avoid in `roder-tui/src/app.rs:216` — `tokio::spawn` is fired from inside the keyboard branch with no way to surface errors or completion as an event. Convert it: the key handler emits `Action::SendTurn { text }`, the update step issues the RPC as a spawned task, and the task's result returns as an `AppEvent`.

### Render popups with `Clear` first

Ratatui widgets do not always blank every cell they cover, so popups bleed through. Always render `Clear` to the popup `Rect` before the popup's own widgets:

```rust
f.render_widget(Clear, popup_area);
f.render_widget(popup_block, popup_area);
```

For centering popups, prefer the modern `Flex` layout (`Layout::horizontal([Constraint::Length(w)]).flex(Flex::Center)` chained with a vertical equivalent) over the older three-way split-and-take-middle trick. Manage stacked popups with an explicit `PopupStack` (`Vec<PopupKind>`) — `gitui` does this so Esc unwinds one level at a time.

### Compose text via `Span → Line → Text`, not `format!`-into-`Paragraph`

Build styled output by composing spans into lines into a `Text`. `Paragraph::new(text)` accepts anything `Into<Text>`, so `"hi".yellow()` (via the `Stylize` trait) is enough for one-off spans. Use `.wrap(Wrap { trim: true })` only when you actually want wrapping; long-running transcript views generally want **explicit line breaks plus a vertical scroll offset** rather than `Wrap`, which recomputes wrapping on every draw and is O(text length).

For anything resembling a text editor (composer with multi-line input, cursor, selection, undo) reach for `tui-textarea`. The hand-rolled `String + push/pop` composer in `roder-tui/src/app.rs:185-189` is fine as a placeholder, but it can't handle paste, multi-line, Unicode width, or cursor movement.

### Theming and color: route through one module

Don't sprinkle `Color::Rgb(...)` or named colors across render code. Define a `Theme` struct with semantic roles (`text`, `text_muted`, `accent`, `border`, `error`, `header_bg`, etc.) and pass it (or store on `App`) so the whole UI restyles in one place. This is exactly what gode's existing `tui-theming` skill enforces on the Bubble Tea side — mirror that discipline in ratatui code. Avoid hard-coding `Color::White`/`Color::Black`; on light terminals they invert. Prefer terminal-default (`Color::Reset`) for primary text unless you have a real reason.

### State + async + cancellation

- Use `tokio_util::sync::CancellationToken` to shut down the event loop, background streams, and in-flight requests in one place.
- For long-running work owned by a component, store a `JoinHandle` on the component and `abort()` it on `Drop` or on state transitions that obsolete the result.
- Subscribe to server/bus events with a `broadcast::Receiver` or `mpsc::Receiver` and pump them into the same `Event` channel — do **not** poll `try_recv()` inside the draw loop (current pattern at `app.rs:229`). Drained-in-draw means events stall behind keystrokes and rendering races state.

### Layout, resize, and constraints

- Define layouts top-down with `Layout::vertical([...])` / `Layout::horizontal([...])` and `Constraint::{Length, Min, Max, Percentage, Ratio, Fill}`. Prefer `Length`/`Min` for chrome (header, footer, borders) and `Min(1)`/`Fill(1)` for the growable area.
- Cache `Layout` only if it shows up in profiles; ratatui's constraint solver is cheap.
- Recompute layout every draw — never store `Rect`s across frames. Terminal size changes arrive as `Event::Resize`; the next draw picks the new size from `frame.area()` automatically.
- Reserve one row for a status/footer that always shows mode + key hints; users navigate TUIs by reading that line.

### Common ecosystem crates worth knowing

- `tui-textarea` — multi-line input widget with editing, paste, selection.
- `tui-input` — single-line input.
- `tui-tree-widget` — tree view (file managers, JSON viewers).
- `tui-big-text` — splash/banner text.
- `ratatui-image` — render images via Kitty/Sixel/iTerm2 protocols where supported.
- `throbber-widgets-tui` — spinners.
- `tui-logger` — in-app log panel hooked into `log`/`tracing`.
- `color-eyre` + `better-panic` — pretty error reporting; install panic hooks **before** entering the alternate screen so a panic does not leave the terminal scrambled.

### Terminal lifecycle hygiene

- Wrap enter/leave alternate screen + raw mode in a guard type with a `Drop` impl, or install a panic hook that runs `disable_raw_mode()` + `LeaveAlternateScreen` before unwinding. The current `roder-tui` does these inline at the end of `run()` — if anything between them panics, the terminal is left broken.
- Enable bracketed paste (`EnableBracketedPaste`) and focus change reporting (`EnableFocusChange`) explicitly if you want `Paste`/`FocusGained`/`FocusLost` events.
- Restore the terminal **before** printing anything on shutdown (errors, summaries) so output goes to the real screen, not the alternate one.

### Pitfalls to avoid

- Don't redraw less than you need to, but don't try to be clever about "dirty rects" either — ratatui already diffs the previous buffer against the new one. Just call `terminal.draw(...)` on each `Render` tick.
- Don't `event::read()` from inside `update`/`view`. Events come from the event task only.
- Don't allocate large strings every draw (e.g., joining all transcript messages into one `String`). Build a `Vec<Line<'static>>` once when messages mutate and pass a slice to `Paragraph` / `List`.
- Don't reach for `mouse` events until keyboard navigation is solid — most TUI users disable mouse capture.
- Don't use `cargo run` for visual sanity — pipe through `gode-tmux-capture` (see related skill) or run interactively.

## Reference apps to read when stuck

When you have a concrete question ("how do I do a fuzzy finder popup?", "how does a tab bar carry focus?"), open the source of the closest analogue rather than guessing:

- **gitui** — multi-tab app, popup stack, queue-based actions, async git via `git2`/`gitoxide`. Strong reference for *anything* keyboard-driven and modal.
- **bottom** — heavy data refresh, charts, customizable layout via config. Reference for tick-driven data widgets.
- **slumber** / **ATAC** — request-builder UIs, forms, navigation between many input fields. Reference for form-heavy screens.
- **yazi** — async I/O file manager, image preview, plugin system. Reference for highly async UIs.
- **spotify-player** — long-running streaming events, OAuth flows surfaced in the UI.
- **kdash** / **gobang** — table-heavy CRUD-style screens.

The `ratatui/templates` repo (`simple`, `event-driven`, `component`) is the canonical starting point — `component` is the closest match to what `roder-tui` should grow into.

## Verification

- After non-trivial UI changes, run the relevant crate tests: `cargo test -p roder-tui` and `cargo clippy -p roder-tui --all-targets -- -D warnings`.
- For visual changes, capture a session via the `gode-tmux-capture` skill and inspect contrast on a light terminal — see the `tui-theming` skill for the contrast rules.
- For event-handling changes, write a unit test that drives a sequence of `Event`s through `update`/`handle_action` and asserts on model state; this is the payoff of keeping `update` pure.
