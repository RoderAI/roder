---
name: roder-theming
description: Use when editing TUI rendering code, adding new visible surfaces, or working on themes/CSS in Roder. Keeps class/id taxonomy, cascade order, and the supported CSS subset consistent across sessions.
---

# Roder TUI theming — alignment rules

The TUI styling pipeline is a small CSS engine (`roder-theme`) plus a thin TUI integration (`roder-tui/src/theme/`). The rules below keep every contributor — human or model — pointed at the same contract.

## 1. Taxonomy: every visible surface registers a token

Single-instance surfaces use an **id**. Repeating surfaces use a **class** (or two: a base class plus a variant).

- Naming: `kebab-case` for both, e.g. `#status-line`, `.timeline-tool`, `.tool-status-error`.
- Variants use a `-variant` suffix on the base class: `.diff-line-add`, `.diff-line-del`.
- Data attributes carry runtime state: `[data-status="error"]`, `[data-mode="plan"]`, `[data-tool="bash"]`.
- The closed list of supported ids/classes is documented in the RFC (`rfc/0001-tui-css-theming.md` §"Class/ID Registry") and enforced at the renderer call sites.
- Hidable classes (those whose `display: none` is honored today) are in `crates/roder-tui/src/theme/overrides.rs` `HIDABLE_CLASSES`. Adding a new entry means teaching the renderer that owns it to consult the flag.

When you introduce a new surface, register its token at the call site — do not invent one and forget to add it to the registry.

## 2. Render through the engine, not through inline colors

Every span SHOULD eventually flow through `StyleMap::resolve(&node)` (or, for the v1 proof, `Theme::for_terminal_themed()` and the conventional `:root` variables in `crates/roder-tui/src/theme/overrides.rs::KNOWN_VARS`). Do not reach back to `Style::default().fg(some_hardcoded_color)` for new code.

Before / after pattern (see `crates/roder-tui/src/app.rs` around line 1737 for current `theme.accent()` style):

```rust
// Wrong — inline color, untheme-able:
let span = Span::styled(" roder", Style::default().fg(Color::Rgb(0xff, 0xaa, 0x00)));

// Right — pulls from the cascade via the Theme helpers:
let span = Span::styled(" roder", self.theme.accent());
```

The Theme struct's helpers (`accent`, `error`, `tool`, ...) are the bridge between the engine and ratatui. New helpers should always derive their fg/bg from `Theme` fields that the cascade already patches.

## 3. Cascade order (lowest → highest priority)

1. **Built-in defaults** — `Theme::for_terminal()` (hard-coded baseline).
2. **Extension contributions** — `ProvidedService::ThemeContribution` (RFC §Extension Contributors; not yet wired).
3. **Active user theme** — first `.css` matched from `~/.roder/themes/`, then `.roder/themes/`, then repo `themes/` (later wins by basename dedup).
4. **Overlays** — extra stylesheets in `[tui.theme].overlays` (not yet wired).
5. **Inline** — `[tui.theme].inline` block (not yet wired).

Tie-break inside a layer is standard CSS specificity `(id, class+attr+pseudo, type)` then author order then `!important`.

**Policy bypass is forbidden.** `display: none` on a permission dialog hides the *render* only; the canonical approval channel still fires (§15 of the whitepaper). Any new "hide a thing" mechanism must respect this — never gate a side effect on a render flag.

## 4. Supported CSS subset (today, v1 proof)

Source of truth: `crates/roder-theme/src/parser.rs`, `crates/roder-theme/src/properties.rs`, `crates/roder-theme/src/cascade.rs`.

### Selectors that match
- `#id`, `.class`, `[data-foo]`, `[data-foo="bar"]`
- Comma groups: `.a, .b { ... }`
- Descendant combinator: `#a .b`
- Child combinator: `#a > .b`
- `:root { --var: value; }` for variables, `var(--name)` to reference
- Specificity + `!important`

### Properties that actually do something
- `color`, `background-color`, `background` (alias for `background-color`)
- `border-style` (`none`/`solid`/`plain`/`single`/`rounded`/`double`/`thick`), `border-radius` (integer; 0 = Plain, >0 = Rounded), `border` shorthand, `border-color`
- `font-weight: normal | bold`
- `font-style: normal | italic`
- `text-decoration: none | underline | line-through`
- `display: block | inline | none` (only `none` has a runtime effect today, and only on `HIDABLE_CLASSES`)
- `padding` (1–4 cell-unit values)

### Values
- Colors: `#rgb`, `#rrggbb`, `rgb(r,g,b)`, `ansi(0..=255)`, named colors (small table — see `properties::named_color`), `reset`, `transparent` (alias of `reset` — only meaningful on a background)
- Lengths: integer cells only

### Terminal background (`:root` / `#body`)

A theme can paint the whole frame with one of three equivalent forms — all collapse to `ThemeOverrides::background` via `resolve_body_background` in `crates/roder-tui/src/theme/overrides.rs`:

- `:root { background: <color>; }` — preferred shorthand.
- `:root { --background: <color>; }` — variable form; composes with `var()`.
- `#body { background-color: <color>; }` — "DOM" form. Wins over the `:root` forms.

`transparent` / `reset` resolve to `None`, which means *do not paint* (terminal native background bleeds through). That is the documented default — `themes/default.css` carries `:root { background: transparent; }` explicitly so the intent is visible. The renderer skips the fill widget entirely when `Theme::body_background` is `None`, so transparent terminals stay truly transparent.

### Popup/dialog surface is always the body

`Theme::apply_overrides` (in `crates/roder-tui/src/app.rs`) unconditionally sets `dialog_bg = body_background.unwrap_or(Color::Reset)` and `dialog_shadow = body_background.unwrap_or(Color::Reset)`. Themes **cannot** elevate the popup interior above the body — the design intent is that popups read as a framed cutout of the body, not as an elevated card. `--dialog-bg` and `--dialog-shadow` declarations in a stylesheet are deliberately ignored. Only `--dialog` (border color) and `--dialog-key-bg` (hotkey chips on confirm dialogs) are themable.

If you find yourself wanting popup elevation, push back on the design before adding it — the "always flush with body" rule fixed two separate visual artifacts (mismatched corners on dark themes, dark popup on light terminals).

### Border shape (`:root` / `#body`)

`:root` / `#body` declarations of `border-style`, `border-radius`, `border` shorthand, and `border-color` resolve via `resolve_border_shape` in `crates/roder-tui/src/theme/overrides.rs` into `ThemeOverrides::border_shape: Option<roder_theme::BorderShape>`. `Theme::apply_overrides` maps that to:

- `Theme::border_type: ratatui::widgets::BorderType` (default `Rounded`).
- `Theme::borders_visible: bool` (false only for `border-style: none`).

Every framed widget reads both fields (`Block::default().borders(borders).border_type(theme.border_type)`). To add per-widget overrides later, the renderer must move from `theme.border_type` to `style_map.computed(node).border` — wire it through one widget at a time, don't try to refactor the whole TUI at once.

### Parsed but inert (do not rely on these — they silently no-op)
- `:hover`, `:focus`, `:selected`, `:first-child`, `:last-child`, `:nth-child`, `:not(...)` — the parser accepts them but matching is incomplete.
- `margin`, `visibility`, `opacity`, `text-transform`, `white-space`, `::before` / `::after`, `+` (adjacent sibling).
- Anything not in the list above.

If you need one of these, **extend the parser and the cascade in lockstep** — don't ship a theme that depends on a property the engine drops.

## 5. Adding a new theme

1. Create `themes/<name>.css`. Set every variable you want to differ from baseline (see `themes/default.css` for the full list).
2. Target at least one class or id beyond `:root` so the cascade is exercised (e.g. `.error { color: ...; }` or `#status-line { ... }`).
3. Register the basename in `crates/roder-tui/tests/theme_e2e.rs::ALL_THEMES`. The `all_checked_in_themes_parse` and `repo_themes_directory_has_ten_or_more_files` tests will fail if you forget.
4. Add a one-line assertion test in the same file that proves the theme changes a specific color or hides a specific class.

The user-facing theme picker discovers any `*.css` under `~/.roder/themes/`, `.roder/themes/`, or the repo `themes/` directory; the basename becomes the picker id. Selection persists to `~/.roder/state.toml` (`[tui].active_theme`).

## 6. What NOT to do

- Don't add inline `Color::Rgb(...)` calls in renderers. Add a `Theme` field, patch it from a conventional `:root` variable (extend `KNOWN_VARS` if needed), and pull it through a `Theme` helper.
- Don't introduce a new CSS property keyword without extending both `crates/roder-theme/src/parser.rs` (lexer) and `crates/roder-theme/src/cascade.rs::apply_declaration` (interpreter). A keyword without an interpreter is a silent footgun.
- Don't ship a theme that depends on `:hover`, `border`, `margin`, `opacity`, or pseudo-elements — they are parsed-but-inert (see §4).
- Don't gate any policy or approval behavior on a render flag like `Theme::hide_thinking`. Render flags are presentation only.
- Don't add a `themes/<name>.css` without also extending the `ALL_THEMES` test list.

## 7. Pointers

- RFC: `rfc/0001-tui-css-theming.md`
- Engine crate: `crates/roder-theme/src/{parser,cascade,properties,style_map,node,ast}.rs`
- TUI integration: `crates/roder-tui/src/theme/{mod,discovery,overrides,state,node_tree}.rs`
- Theme baseline / palette glue: `crates/roder-tui/src/app.rs` `impl Theme` (around lines 110–330)
- Palette source: `crates/roder-tui/src/palette/sources.rs::theme_source` + `PaletteAction::SetTheme`
- Tests: `crates/roder-tui/tests/theme_e2e.rs`, `crates/roder-tui/tests/theme_palette.rs`, `crates/roder-theme/src/cascade.rs` (unit tests)
- Bundled themes (10 today; `default`, `midnight`, `solarized`, `minimal`, `high-contrast`, `gruvbox`, `dracula`, `tokyo-night`, `light`, `focus`) live in `themes/` — keep the on-disk set and `ALL_THEMES` in `tests/theme_e2e.rs` in sync.
