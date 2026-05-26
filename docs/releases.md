# Roder Releases

Roder releases are managed by release-plz.

The workspace currently runs release-plz in git-only mode. That means
release-plz opens version-bump release PRs and creates git tags/GitHub releases,
but it does not publish crates to crates.io.

## Workflow

The `.github/workflows/release-plz.yml` workflow runs after pushes to `main` or
`master`, and can also be started manually from GitHub Actions:

1. `release-plz release` runs first. With `release_always = false`, it only
   releases when the pushed commit came from a release-plz release PR.
2. `release-plz release-pr` then opens or updates the release PR for unreleased
   Cargo workspace changes.

Normal feature PRs do not need manual version edits. After they merge,
release-plz prepares the version bump in its own PR. Merging that release PR is
the release gate.

## Configuration

The repository-level config is `release-plz.toml`:

- `git_only = true` keeps releases tag-based until crates.io publication is
  ready.
- `release_always = false` prevents every merge to `main` from immediately
  publishing a release.
- `version_group = "roder"` keeps first-party workspace crates on one shared
  version line.
- `changelog_update = false` avoids creating per-crate changelog files for the
  large internal workspace.
- `semver_check = false` avoids cargo-semver-checks while the crates are still
  unpublished and pre-1.0.

When adding a first-party crate, add a matching `[[package]]` entry with
`version_group = "roder"` so release-plz keeps it on the shared workspace
version line.

## GitHub Setup

The repository must allow GitHub Actions to create pull requests:

Settings -> Actions -> General -> Workflow permissions -> Read and write
permissions, with pull request creation enabled.

The workflow uses only `GITHUB_TOKEN` while `git_only = true`. Do not add a
`CARGO_REGISTRY_TOKEN` until crates.io publication is intentionally enabled.

## Crates.io Readiness

Before enabling crates.io publication:

1. Add publish metadata to crates that should be public: description, license or
   license file, repository, and README policy.
2. Add registry versions to internal workspace dependencies, for example:

   ```toml
   roder-api = { path = "crates/roder-api", version = "0.1.0" }
   ```

3. Mark private crates with `publish = false`.
4. Enable release-plz publishing and registry authentication.
5. Revisit `semver_check` package by package once public API compatibility is a
   release blocker.

## Homebrew

The existing Homebrew helper remains manual and should be run from an already
settled release version:

```sh
VERSION=0.1.1 make release-brew
```
