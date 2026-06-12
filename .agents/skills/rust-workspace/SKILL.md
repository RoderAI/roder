---
name: rust-workspace
description: Runs and extends the Gode/Roder Rust workspace using mise, lld, Cargo incremental builds, and Makefile test targets. Use when adding crates or dependencies, running Rust tests, building roder-cli, or configuring local Rust tooling in this repo.
---

# Rust workspace (Gode / Roder)

## Environment

Work from the repo root with **mise** active (`rust 1.95.0`, `uv`, `python`).

```sh
cd /path/to/gode   # mise enter hook installs brew lld if missing
mise run rust:setup   # first time: lld and nextest
```

**First-time setup:** `mise run rust:setup` (or `make dev-deps`).

Local build acceleration (already configured; do not duplicate in per-crate files):

| Piece | Location |
|-------|----------|
| `lld` (macOS) | mise `PATH` + `.cargo/config.toml` → `clang` + `-fuse-ld=lld` |
| Dev profiles | root `Cargo.toml` → `debug = 0`, `split-debuginfo = "unpacked"` |
| Incremental | `.cargo/config.toml` → `incremental = true` |

Do not enable `sccache` by default for local dev: it can defeat incremental rebuild behavior and make edit/build loops slower. Use `RUSTC_WRAPPER=sccache ...` only for deliberate clean/shared-cache builds.

Optional roadmap experiments: `CARGO_TARGET_DIR=.target-roadmap-local` (gitignored).

### Artifact directory lock (`Blocking waiting for file lock`)

Usually another job holds `target/` — often **rust-analyzer** (`cargo check`), not a visible terminal process.

```sh
pgrep -fl 'gode/target|cargo.*roder'
```

- Wait for it to finish, or reload the Cursor window / pause rust-analyzer.
- This repo sets `rust-analyzer.cargo.targetDir` → `target/rust-analyzer` (`.vscode/settings.json`) so IDE checks do not fight `make build`.
- If nothing is running and the lock is stale: `make cargo-unlock` (see `make cargo-unlock-help`).

## Running tests

Prefer **Makefile / mise** over ad-hoc full-workspace `cargo test`.

| Goal | Command |
|------|---------|
| Day-to-day (unit tests only) | `make test-fast` or `mise run rust:test:fast` |
| Pre-push / CI parity | `make test` or `mise run rust:test` |
| One crate | `cargo test -p roder-core` |
| One crate, lib only | `cargo test -p roder-core --lib` |
| App-server JSON-RPC e2e | `cargo test -p roder-app-server --features e2e-tests` (included in `make test`) |
| Format / clippy | `mise run rust:fmt`, `mise run rust:clippy` |

`make test` uses **cargo-nextest** when installed; otherwise `cargo test`. Config: `.config/nextest.toml`.

**Live / network tests** use `#[ignore]` and env vars (`RODER_LIVE_*`, API keys). Do not run ignored tests unless the user explicitly opts in with the documented env vars.

**Compile-only check** while editing: `cargo check -p <crate>` (faster than `build`).

## Adding dependencies

### External crates (crates.io)

1. Add the version (and shared features) to **`[workspace.dependencies]`** in the **root** `Cargo.toml`.
2. In the target crate: `some-crate.workspace = true` (add extra features only if this crate needs more than the workspace default).
3. Run `cargo check -p <target-crate>`.

**Do not** `cargo add` into a leaf crate without updating the workspace root for shared deps — that duplicates versions and slows builds.

**Defaults:**

- `tokio` — use `tokio.workspace = true` only; workspace already trims features (no `full`).
- `serde` — `serde = { workspace = true, features = ["derive"] }` when derive is needed.
- Prefer `default-features = false` on heavy deps at the workspace level when possible.

### Internal `roder-*` crates

1. Add `roder-foo = { path = "crates/roder-foo" }` under **`[workspace.dependencies]`** in root `Cargo.toml` (if new crate).
2. Add `crates/roder-foo` to workspace members (glob `crates/*` usually picks it up).
3. In dependents: `roder-foo.workspace = true`.
4. Run `cargo check -p roder-foo`.

## New workspace crate checklist

- [ ] `crates/<name>/Cargo.toml` with explicit `version = "0.1.0"` (crates are versioned per-package; do NOT use `version.workspace = true`) and `edition.workspace = true`
- [ ] Root `Cargo.toml` `[workspace.dependencies]` entry for the package path
- [ ] Regenerate the knope release config: `python3 scripts/generate-knope-config.py` (CI fails if `knope.toml` is stale)
- [ ] Add a `.changeset/*.md` changeset for the PR (see [changesets](../changesets/SKILL.md) and `docs/releases.md`)
- [ ] `cargo check -p <name>` then targeted tests

## Building the product binary

```sh
make build          # bin/roder debug
make run-existing   # run bin/roder without invoking cargo
make install        # release → ~/.local/bin/roder
cargo build -p roder-cli --bin roder
```

Main binary crate: **`roder-cli`** (pulls app-server, tui, extensions).

## Related skills

- Clippy fixes: [rust-clippy](../rust-clippy/SKILL.md)
- App-server API docs: [roder-app-server-docs](../roder-app-server-docs/SKILL.md)

## Repo rules (agents)

Per `AGENTS.md`: ignore unfamiliar in-flight work from other agents; no backwards-compat shims for APIs meant to move forward; split files that grow past ~500 lines.
