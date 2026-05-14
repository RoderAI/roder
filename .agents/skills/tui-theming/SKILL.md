---
name: tui-theming
description: Keep gode TUI colors readable across dark and light terminals. Use when changing Bubble Tea, Lip Gloss, Glamour markdown, transcript, composer, dialog, footer, or other terminal UI rendering.
---

# TUI Theming

## Instructions

- Route new TUI colors through `internal/tui/components/theme.go`; do not add hard-coded white, black, or gray `lipgloss.Color(...)` calls in render components.
- Preserve dark-theme intent, then choose a light-theme counterpart with real contrast. Primary text must be dark on light terminals and light on dark terminals.
- Prefer semantic roles such as `ColorText`, `ColorTextStrong`, `ColorMuted`, `ColorAccent`, `ColorBorder`, `ColorTool`, and `ColorError` over one-off color indexes.
- Auto-detected terminal theme is applied at TUI startup through `components.DetectAndSetTheme()`. If adding a new non-fullscreen TUI surface, call it before constructing styles.
- If rendered output is cached, include `components.ThemeVersion()` in the cache key or clear the cache when the theme changes.
- For Glamour markdown rendering, use the active theme colors as the base style and avoid code-block backgrounds unless they are explicitly theme-aware.
- Add focused tests for light terminals when touching colors. Assert that primary text does not render as white-ish ANSI colors (`231`, `252`) in light theme.

## Verification

- Run the package tests for the changed TUI component.
- Run `go test ./internal/tui ./internal/tui/components -count=1` for broad TUI style changes.
- Use the `gode-tmux-capture` skill for visual proof when changing layout, contrast, or interaction-heavy rendering.
