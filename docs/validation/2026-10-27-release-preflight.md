# Runtime compatibility release preflight

The release candidate is isolated from the original dirty development worktree.
It contains the seven requested runtime/provider/TUI improvements and their
regression coverage, not the unrelated configured-model picker or Markdown-table
work. Original worktree file contents were retained unchanged.

## Compatibility boundary

The shared Rust API removes `ToolExecutor::supports_eager_execution` in favor of
`execution_mode` and adds public configuration/ownership fields. This requires
a pre-1.0 compatibility boundary, not a patch-compatible API release.

The release changeset marks all 96 packages in the API's reverse dependency
closure as `major`, including `roder`, whose public library exposes extension
types. Expected CLI version: `0.4.0`; API: `0.3.0`; core: `0.4.0`.

Six independent packages are unchanged: `roder-api-transcript`, `roder-edit-core`,
`roder-edit-wasm`, `roder-search`, `roder-theme`, and `roder-web-search`.
The dependency graph, including development dependencies, has no cycles.

## Scoped source validation

- Full TUI: 659 passed, zero failures.
- Full core and Responses: 629 passed, zero failures, three explicitly live
  tests ignored.
- `make registry-readmes`: passed.
- `make fmt` and `make fmt-check`: passed.
- Knope generation and whitespace checks: passed.

The earlier implementation report tested the original combined development tree
and consequently reports ten additional unrelated TUI tests. This release
candidate's own counts above are authoritative for the scoped source.

## Publication procedure

Merge the implementation PR carrying changesets, let `prepare-release.yml`
prepare versioned manifests/changelogs, and review/merge that release PR.
Registry publication follows the immutable release commit, in full Cargo
dependency order, using `scripts/publish-crates.py`.

GitHub tagging, signed binaries, npm CLI publication, and Homebrew follow the
existing release workflow. Existing published crates are immutable; only the
new release versions may be uploaded. Verify registry versions and workflow
artifacts rather than equating a successful merge with successful publication.
