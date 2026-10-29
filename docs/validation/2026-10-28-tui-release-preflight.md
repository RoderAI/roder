# TUI release preflight

The local development snapshot was preserved on `backup/local-feature-snapshot`
and rebased onto `cdeb5aaa` (`origin/master`). The runtime recovery, compaction,
tool scheduling, provider capabilities, owned worktrees, image previews, source
copy, and discovery-startup work was already merged and released upstream.
Consumed changesets for that work were not reintroduced.

The remaining release contains:

- Configured-provider filtering in the model menu and command palette, retaining
  the active provider, routing choices, and fresh-install fallback.
- Markdown tables with aligned columns and a narrow-terminal stacked layout.
- Grapheme-aware table fitting, wrapping, padding, and Unicode regressions.
- Restoration of the streaming regression test's missing `#[test]` attribute.

The CLI and TUI changesets request patch releases so distributed binaries include
the new TUI behavior.

## Frozen-source checks

- TUI: 674 tests passed.
- API, core, and Responses: 933 passed, with live tests ignored and the existing
  `failed_runner_initialization_is_retryable_without_local_fallback` failure
  explicitly excluded. That failure was reproduced on unmodified upstream
  master in a separate detached worktree.
- App-server E2E: 140 passed, one live test ignored. The inherited
  `CURSOR_API_KEY` was unset for the test process; otherwise it invalidates the
  test's assumption that Cursor starts unauthenticated.
- CLI debug build, formatting, knope generation, and whitespace checks passed.
- Strict Clippy found existing warnings in unchanged API/TUI code. The targeted
  TUI check passed with only those baseline lint categories allowed:
  `large_enum_variant`, `manual_div_ceil`, `unnecessary_lazy_evaluations`,
  `type_complexity`, and `assertions_on_constants`.
- The broader ACP suite has six existing CUA failures. A representative failure
  was reproduced on unmodified upstream master; no ACP files changed here.

The shared `/tmp` filesystem filled during validation. Subsequent tests used
`$HOME/.cache/roder-release-validation/tmp`; sccache was bypassed for the final
builds after its temporary-file operations failed.

An isolated-config tmux smoke run opened the model menu and showed its
"Configured models" title and active mock model. Text captures are under
`.roder/tmux-captures/release/`; PNG rendering was unavailable because Pillow
was not installed. Detailed local test/build logs are under
`.roder/release-validation/`.
