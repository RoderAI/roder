## 0.1.2 (2026-08-03)

### Features

#### Add `/review`: a read-only review sub-turn with structured findings and pluggable review publishers

`/review` runs a detached, read-only reviewer over the working diff, a base
branch, a commit, or a free-form scope, and returns prioritized findings with
file/line locations. Findings render in a new TUI panel where they can be kept
or dropped, and are exposed over the app-server as `review/start`,
`review/publish`, and `review/publishers/list` plus `review/started`,
`review/completed`, `review/failed`, and `review/published` notifications.

Publishing goes through a new `ReviewPublisher` extension service. The
first-party `roder-ext-github-review` publisher submits findings as GitHub pull
request review comments over the `gh` CLI or the REST API, with diff-hunk
prefiltering and a dry-run mode. Configure it under `[review]` and
`[review.publishers.github]`. Each `[review.publishers.<id>]` block is stored
opaquely and parsed by the publisher's own crate, so adding a platform does not
change the core config types.

Also fixes `roder app-server`, which built its Tokio runtime in current-thread
mode. Providers that bridge a synchronous callback back into async work call
`tokio::task::block_in_place`, which panics outright on a current-thread
runtime, so the `claude-code` provider aborted the server on its first
Roder-executed tool call. The app-server now uses a multi-threaded runtime like
the TUI entry point.

## 0.1.1 (2026-06-15)

### Features

#### One-command Roder package install (`roder install npm:/git:/path`)

Roder packages bundle process extensions, skills, slash commands, and themes
behind a root `roder.toml` manifest. Install from npm, git (shorthand, SSH,
raw URLs, pinned refs), or local paths; manage with `roder packages
list|resources|enable|disable|approve|filter|sync|init`, `roder remove`,
`roder update`, and ephemeral `-e` loading. Resources surface through the
existing skills/commands/theme registries; the process-extension protocol
gains manifest-declared tool providers served over `tools/call`. New
app-server `packages/*` methods, a `/packages` builtin, and a Packages
palette section round out the surfaces. npm lifecycle scripts stay disabled
unless `--allow-scripts` is passed, and package process extensions never
launch before explicit approval.

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
