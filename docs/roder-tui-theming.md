# Roder TUI Theming

Roder ships a small CSS engine that lets you restyle every visible surface in the TUI by dropping a `.css` file into `~/.roder/themes/`. Pick the active theme from the Ctrl+P menu under **Themes**; the screen restyles on the next frame.

The full design lives in [`rfc/0001-tui-css-theming.md`](../rfc/0001-tui-css-theming.md). This page is the user-facing API reference: where files go, what selectors and properties work today, and how the cascade resolves.

---

## Quick start

```sh
mkdir -p ~/.roder/themes
cat > ~/.roder/themes/my-theme.css <<'CSS'
:root {
    --accent: #ff8a00;
    --error: #d96666;
}

.timeline-thinking { display: none; }
.error             { color: var(--error); font-weight: bold; }
CSS
```

Launch `roder`, press `Ctrl+P`, choose **Themes**, select `my-theme`. The active choice persists to `~/.roder/state.toml` so the next launch picks it up.

You can also pin a theme without the menu:

```sh
ROADER_THEME=midnight cargo run -p roder
```

---

## Where themes live

| Path                          | Purpose                                                      |
| ----------------------------- | ------------------------------------------------------------ |
| `~/.roder/themes/*.css`       | Your personal themes.                                        |
| `.roder/themes/*.css`         | Project-local themes. Override user themes with the same basename. |
| `<repo>/themes/*.css`         | Bundled defaults shipped with Roder.                         |
| `~/.roder/state.toml`         | Persisted active selection. Written when you commit a choice in the picker. |

Discovery dedups by basename — a `.roder/themes/midnight.css` shadows a `~/.roder/themes/midnight.css` of the same name.

### Selection precedence

1. `ROADER_THEME` environment variable
2. `[tui] active_theme` in `~/.roder/state.toml` (written by the picker)
3. `[tui.theme] active` in `~/.roder/config.toml`
4. `"default"` if it exists, otherwise the compiled-in baseline

---

## Bundled themes

The repo ships 10 themes you can copy or fork:

`default`, `midnight`, `solarized`, `minimal`, `high-contrast`, `gruvbox`, `dracula`, `tokyo-night`, `light`, `focus`.

Start by reading `themes/default.css` — it declares every variable Roder understands. Override only the ones you care about; the rest fall back to the compiled-in palette.

---

## Selectors

```css
#id                 /* single-instance surface (e.g. #status-line)              */
.class              /* repeating surface (e.g. .timeline-tool)                  */
[data-foo]          /* runtime state attribute                                  */
[data-foo="bar"]    /* attribute equals                                          */
.a, .b              /* comma group                                              */
#a .b               /* descendant                                               */
#a > .b             /* direct child                                             */
:root               /* variable declarations                                    */
```

`:hover`, `:focus`, `:selected`, `:first-child`, `:last-child`, `:nth-child`, `:not`, and the `+` adjacent-sibling combinator are **parsed but inert** today — they will not match anything. Don't rely on them.

### Specificity

Standard CSS tuple `(id, class+attr+pseudo, type)`. Ties resolve by author order. `!important` is honored but should be rare.

---

## Properties

| Property            | Values                                                  | Notes                                                  |
| ------------------- | ------------------------------------------------------- | ------------------------------------------------------ |
| `color`             | any color value (see below)                             | Foreground.                                            |
| `background-color`  | any color value                                         |                                                        |
| `background`        | any color value (alias of `background-color`)           | Useful on `:root` and `#body` to paint the whole frame. See "Terminal background" below. |
| `font-weight`       | `normal` \| `bold`                                      |                                                        |
| `font-style`        | `normal` \| `italic`                                    | Italics depend on terminal support.                    |
| `text-decoration`   | `none` \| `underline` \| `line-through`                 |                                                        |
| `display`           | `block` \| `inline` \| `none`                           | Only `none` has runtime effect today, and only on `HIDABLE_CLASSES` (see below). |
| `padding`           | 1–4 integer cell counts                                 | Parsed; renderer hookup pending.                       |
| `border-style`      | `none` \| `solid`/`plain`/`single` \| `rounded` \| `double` \| `thick` | Applies globally to every framed widget. See "Borders" below. |
| `border-radius`     | integer cell count                                      | `0` → square corners (Plain); any positive value → Rounded. |
| `border-color`      | any color value                                         | Currently honored via the existing `--border` palette variable. |
| `border`            | shorthand: `<style>` and/or `<color>` in any order      | `border: rounded`, `border: thick #f80`, `border: none`. |

`margin`, `visibility`, `opacity`, `text-transform`, `white-space`, `::before` / `::after`, and any property not in the table above are **parsed but inert**. Adding them requires extending both the parser and the cascade interpreter — see the skill at `.claude/skills/roder-theming/SKILL.md`.

### Color values

```css
color: #fff;                  /* short hex                                      */
color: #6ccaff;               /* hex                                            */
color: rgb(108, 202, 255);    /* rgb()                                          */
color: ansi(212);             /* 0..=255 terminal palette                       */
color: red;                   /* named: red, green, blue, yellow, cyan, magenta */
                              /*        black, white, gray, darkgray            */
                              /*        lightred/green/yellow/blue/magenta/cyan */
                              /*        orange, purple, pink, silver            */
color: reset;                 /* terminal default                               */
color: transparent;           /* same as `reset` — used on background only      */
color: var(--accent);         /* :root variable                                 */
```

For anything outside the named set, use `#rrggbb`.

---

## Borders

Every framed widget in the TUI (composer, Ctrl+P menu, confirm dialogs, tool detail, diff viewer) shares one global border shape. Themes set it via `:root` or `#body`:

```css
:root { border-radius: 0; }              /* square corners (Plain)               */
:root { border-radius: 1; }              /* rounded corners (any nonzero = same) */
:root { border-style: rounded; }         /* explicit keyword form                */
:root { border-style: double; }          /* ╔══╗ double line                     */
:root { border-style: thick; }           /* heavy single line                    */
:root { border-style: none; }            /* no border drawn at all               */
#body { border: thick #ff8800; }         /* shorthand, equivalent to two decls   */
```

Shape keywords accepted: `none`, `solid` / `plain` / `single`, `rounded`, `double`, `thick` / `heavy` / `bold`.

`#body { ... }` wins over `:root { ... }` (matching the background precedence). Border color comes from the existing `--border` variable; the dedicated `border-color` property is parsed but currently routed through the same color path.

Borders apply globally because every widget currently hardcodes its frame; per-widget overrides (e.g. `#composer { border-style: double }` while everything else stays rounded) need each renderer to read the StyleMap per node — a follow-up beyond the v1 proof.

---

## Terminal background

By default Roder paints nothing behind the TUI — your terminal's own background, transparency, and image bleed through. To opt out, set a background on `:root` or `#body`. The three syntaxes below all do the same thing:

```css
:root { background: #1a1b26; }            /* shortest                                */
:root { --background: #1a1b26; }          /* variable form (composes with var())     */
#body { background-color: #1a1b26; }      /* "DOM" form, equivalent                  */
```

The default theme declares this explicitly so the intent is visible:

```css
:root { background: transparent; }        /* the default — terminal shows through    */
```

`transparent` and `reset` both collapse to "do not paint", so flipping a theme back to transparent is one keyword. If multiple sources set a background, `#body { ... }` wins over `:root { --background: ... }` wins over `:root { background: ... }` (later in cascade order beats earlier).

---

## Variables (`:root`)

Variables are the easiest way to retheme: every value below maps to a compiled-in palette field, so setting just these will retint the whole TUI without touching individual classes.

```css
:root {
    --background:         transparent;   /* terminal native bg (default)            */
    --text:               reset;
    --muted:              ansi(244);
    --subtle:             ansi(245);
    --accent:             ansi(212);
    --accent-soft:        ansi(183);
    --tool:               ansi(244);
    --diff-added:         ansi(114);
    --diff-added-bg:      ansi(22);
    --diff-removed:       ansi(210);
    --diff-removed-bg:    ansi(52);
    --shell:              ansi(220);
    --error:              ansi(196);
    --border:             ansi(244);
    --mode-plan:          ansi(75);
    --mode-default:       ansi(244);
    --mode-accept-all:  ansi(40);
    --mode-bypass:        ansi(196);
    --selection-bg:       ansi(212);
    --selection-fg:       reset;

    /* Dialog / popup. The popup *interior* is always the body color
     * (transparent themes → transparent popup) — only the border color
     * and hotkey-chip background are themable. */
    --dialog:             ansi(62);    /* popup border color                  */
    --dialog-key-bg:      ansi(238);   /* hotkey chip background on dialogs   */
}
```

Unknown variable names are silently ignored. Variables may reference other variables — `--accent: var(--brand);` works, up to a depth of 8.

---

## Class / ID taxonomy

The set of recognized tokens is closed. Selectors that target unknown ids/classes parse fine but never match.

### IDs (single instance)

`#app`, `#composer`, `#status-line`, `#timeline`, `#palette`, `#dialog`, `#diff-viewer`, `#tool-detail`, `#inspector`.

### Classes (repeating)

| Surface     | Classes                                                                                            |
| ----------- | -------------------------------------------------------------------------------------------------- |
| Timeline    | `.timeline-item`, `.timeline-user`, `.timeline-assistant`, `.timeline-thinking`, `.timeline-tool`, `.timeline-system`, `.timeline-error` |
| Tool        | `.tool-header`, `.tool-args`, `.tool-output`, `.tool-diff`, `.tool-status-pending`, `.tool-status-ok`, `.tool-status-error` |
| Palette     | `.palette-source`, `.palette-item`, `.palette-item-selected`, `.palette-title`, `.palette-subtitle`, `.palette-keyword` |
| Status line | `.segment`, `.segment-mode`, `.segment-model`, `.segment-thread`, `.segment-branch`, `.segment-usage`, `.segment-mcp` |
| Diff        | `.diff-file`, `.diff-hunk`, `.diff-line-add`, `.diff-line-del`, `.diff-line-ctx`, `.hunk-pending`, `.hunk-accepted`, `.hunk-rejected` |
| Cross-cut   | `.error`, `.warning`, `.muted`, `.accent`, `.code`, `.kbd`, `.link`                                |

### Data attributes

`[data-status="error|ok|pending"]`, `[data-tool="bash|file_edit|..."]`, `[data-mode="plan|default|accept_edits"]`, `[data-id="<segment-id>"]`.

### `display: none` today

`display: none` only takes effect on classes in `HIDABLE_CLASSES` — currently:

`.timeline-thinking`, `.timeline-system`, `.timeline-error`, `.timeline-tool`, `.timeline-assistant`, `.timeline-user`.

Adding a new hidable surface means extending both `crates/roder-tui/src/theme/overrides.rs::HIDABLE_CLASSES` and the renderer that owns the surface. Until that work lands, `display: none` on other classes is silently dropped.

---

## Cascade order

Lowest to highest priority:

1. **Compiled-in baseline** — `Theme::for_terminal()` in `crates/roder-tui/src/app.rs`.
2. **Active theme** — the file selected by the picker / env var / config.
3. **Overlays** (planned) — extra stylesheets from `[tui.theme] overlays`.
4. **Inline** (planned) — rules from `[tui.theme] inline`.

Inside a layer, tie-break is CSS specificity, then author order, then `!important`.

**Policy is not styleable.** `display: none` on a permission dialog hides the *render* only — the canonical approval channel still fires. Themes can never suppress a side effect.

---

## Cookbook

### Hide thinking blocks

```css
.timeline-thinking { display: none; }
```

### Tone down errors

```css
.error, .timeline-error { color: #d77; font-weight: normal; }
```

### Brand the composer

```css
#composer { background-color: #1a1f2e; padding: 1 2; }
:root     { --accent: #6cf; --border: #6cf; }
```

### A focused-pairing layout

```css
.timeline-thinking,
.timeline-tool       { display: none; }
.timeline-assistant  { color: #fff; font-weight: bold; }
.error               { color: #ff6b6b; }
```

### Dark / light variants from one file

```css
:root {
    --bg:     #1e1e2e;
    --fg:     #cdd6f4;
    --accent: #89b4fa;
}

:root[data-mode="light"] {
    --bg:     #ffffff;
    --fg:     #1c1c1c;
    --accent: #0066cc;
}
```

---

## The picker

Press `Ctrl+P`, navigate to **Themes**, pick a theme, press Enter. The TUI:

1. Reloads the active theme file (so saving and re-picking the same theme refreshes it).
2. Re-cascades against the baseline `Theme`.
3. Persists the choice to `~/.roder/state.toml`.
4. Restyles on the next frame.

The same picker is reachable as a palette source (`PaletteSource` named `themes`) for the future built-in palette UI.

---

## Troubleshooting

| Symptom                              | Likely cause                                                                 |
| ------------------------------------ | ---------------------------------------------------------------------------- |
| Theme doesn't appear in the picker.  | File isn't `*.css`, or lives outside the three discovery directories.        |
| Theme picks but nothing changes.     | The rules target unknown ids/classes, or properties that are parsed-but-inert. Check against the taxonomy above. |
| `display: none` doesn't hide.        | Class is not in `HIDABLE_CLASSES`. See above.                                |
| Parse error.                         | The whole file is rejected. Roder falls back to the previous theme; check `cargo run` stderr for the file:line. |
| Color looks wrong on a 256-color terminal. | `#rrggbb` requires a truecolor terminal. Fall back to `ansi(n)` for older terms. |

---

## See also

- `rfc/0001-tui-css-theming.md` — full design document.
- `.claude/skills/roder-theming/SKILL.md` — contributor rules for adding surfaces and properties.
- `themes/` — 10 bundled themes worth reading as examples.
- `crates/roder-theme/src/` — the engine (parser, cascade, properties).
- `crates/roder-tui/src/theme/` — TUI integration (discovery, overrides, state, picker dispatch).
