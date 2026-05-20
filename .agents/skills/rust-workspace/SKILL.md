---
name: rust-workspace
description: Runs and extends the Gode/Roder Rust workspace using mise, sccache, lld, hakari, and Makefile test targets. Use when adding crates or dependencies, running Rust tests, building roder-cli, or configuring local Rust tooling in this repo.
---

# Rust workspace (Gode / Roder)

## Environment

Work from the repo root with **mise** active (`rust 1.95.0`, `sccache`, `uv`, `python`).

```sh
cd /path/to/gode   # mise enter hook installs brew lld if missing
mise run rust:setup   # first time / after dep changes: lld, nextest, hakari, workspace-hack
```

**First-time / after changing workspace deps:** `mise run rust:setup` (or `make dev-deps` + `make hakari-update`).

Local build acceleration (already configured; do not duplicate in per-crate files):

| Piece | Location |
|-------|----------|
| `sccache` | `.cargo/config.toml` → `rustc-wrapper = "sccache"` |
| `lld` (macOS) | mise `PATH` + `.cargo/config.toml` → `clang` + `-fuse-ld=lld` |
| `workspace-hack` | `workspace-hack/` + `.config/hakari.toml` |
| Dev profiles | root `Cargo.toml` → `debug = 1`, `split-debuginfo = "unpacked"` |

Optional roadmap experiments: `CARGO_TARGET_DIR=.target-roadmap-local` (gitignored).

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
3. Regenerate hakari:

```sh
cargo hakari generate
cargo hakari manage-deps -y
```

Or: `make hakari-update` / `mise run rust:setup` (includes hakari).

**Do not** `cargo add` into a leaf crate without updating the workspace root for shared deps — that duplicates versions and slows builds.

**Defaults:**

- `tokio` — use `tokio.workspace = true` only; workspace already trims features (no `full`).
- `serde` — `serde = { workspace = true, features = ["derive"] }` when derive is needed.
- Prefer `default-features = false` on heavy deps at the workspace level when possible.

### Internal `roder-*` crates

1. Add `roder-foo = { path = "crates/roder-foo" }` under **`[workspace.dependencies]`** in root `Cargo.toml` (if new crate).
2. Add `crates/roder-foo` to workspace members (glob `crates/*` usually picks it up).
3. In dependents: `roder-foo.workspace = true`.
4. Run hakari generate + manage-deps (new crate gets `workspace-hack` via `manage-deps`).

### `workspace-hack`

- Managed by **cargo hakari** — edit only the `BEGIN HAKARI SECTION` via `cargo hakari generate`, not by hand.
- Every workspace crate should keep `workspace-hack = { version = "0.1", path = "../../workspace-hack" }` (path depth varies by crate dir).

## New workspace crate checklist

- [ ] `crates/<name>/Cargo.toml` with `version.workspace = true`, `edition.workspace = true`
- [ ] Root `Cargo.toml` `[workspace.dependencies]` entry for the package path
- [ ] `cargo hakari generate && cargo hakari manage-deps -y`
- [ ] `cargo check -p <name>` then targeted tests

## Building the product binary

```sh
make build          # bin/roder debug
make install        # release → ~/.local/bin/roder
cargo build -p roder-cli --bin roder
```

Main binary crate: **`roder-cli`** (pulls app-server, tui, extensions).

## Related skills

- Clippy fixes: [rust-clippy](../rust-clippy/SKILL.md)
- App-server API docs: [roder-app-server-docs](../roder-app-server-docs/SKILL.md)

## Repo rules (agents)

Per `AGENTS.md`: ignore unfamiliar in-flight work from other agents; no backwards-compat shims for APIs meant to move forward; split files that grow past ~500 lines.
