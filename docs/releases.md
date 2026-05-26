# Roder Releases

Roder uses a lockstep workspace version. Every first-party crate under
`crates/` inherits `[workspace.package].version` from the root `Cargo.toml`, so a
release is one workspace release tagged as `vX.Y.Z`.

This is deliberate while the project is pre-1.0 and the internal crate graph is
still moving quickly. Independent per-crate versions would add merge conflicts
and publish ordering before the public API boundary is stable enough to make
that useful.

## Required Version Bumps

Any release-affecting change must increase `[workspace.package].version` in the
root `Cargo.toml`.

Release-affecting paths are:

- `Cargo.toml`
- `Cargo.lock`
- `crates/**`

The `Version bump` GitHub check runs on pull requests and pushes to `main` or
`master`. Make that check required in branch protection to block merges that
touch Cargo workspace code without a version bump.

Run the same check locally with:

```sh
make release-check BASE_REF=origin/master
```

Use `origin/main` instead if the default branch is renamed.
The local script includes committed, staged, unstaged, and untracked files, so
run it from a worktree that does not contain unrelated crate edits.

## Automatic GitHub Releases

The `Release` workflow runs after a push to `main` or `master` that changes
Cargo workspace files. It:

1. Reads `[workspace.package].version`.
2. Verifies the version increased on push events.
3. Verifies `Cargo.lock` is up to date with `cargo metadata --locked`.
4. Runs `make test`.
5. Creates an annotated `vX.Y.Z` tag if it does not already exist.
6. Creates a GitHub Release with generated notes.

Manual releases use the same workflow through `workflow_dispatch`. Pass the
expected version as a guard, or leave it empty to release the version currently
checked into `Cargo.toml`.

## Bump Rules

Until `1.0.0`, treat the version as a coordination signal first:

- Patch bump: fixes, internal refactors, docs generated from release-affecting
  crate changes, and small behavior changes.
- Minor bump: new user-visible features, new extension surfaces, new app-server
  methods, or material behavior changes.
- Major bump: reserved for post-`1.0.0`.

Because all crates share the workspace version, the largest change in a merge
decides the bump.

## Crates.io Readiness

The current workspace is not ready to publish every crate to crates.io as-is.
Internal dependencies in `[workspace.dependencies]` are path-only, which Cargo
does not permit in published packages. Before enabling cargo registry
publication:

1. Add package metadata needed for published crates, including descriptions,
   license or license file, repository, and README policy.
2. Add registry versions to internal workspace dependencies, for example:

   ```toml
   roder-api = { path = "crates/roder-api", version = "0.1.0" }
   ```

3. Decide which crates are intentionally private and mark them `publish = false`.
4. Add a dry-run publish check for the publishable crate set.
5. Move publication to `release-plz` or a dedicated ordered `cargo publish`
   workflow once the first crate ownership and registry-token setup is complete.

`release-plz` is the right next automation once crates.io publication is real:
it can create release PRs, publish unpublished packages, create tags/releases,
run semver checks for library packages, and run in git-only mode for packages
that should be tagged but not published.

## Homebrew

The existing Homebrew helper remains manual:

```sh
VERSION=0.1.1 make release-brew
```

The helper refuses to run unless `VERSION` matches `[workspace.package].version`.
Use `PUBLISH=1` only after the workspace release tag and GitHub release policy
are settled for the version being published.
