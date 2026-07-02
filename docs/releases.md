# Roder Releases

Releases are managed by [knope](https://knope.tech) using **changesets** with
**per-package versioning**. Every Cargo workspace crate and every SDK package
(`sdk/typescript`, `sdk/python`, `packages/edit-tools`) is versioned
independently. Versions only move when a changeset says so — commit messages
never drive version bumps (`[changes] ignore_conventional_commits = true`).

knope owns version bumps, per-package changelogs, git tags
(`<package>/v<version>`), and GitHub releases. Registry publication is a
separate explicit step for crates.io, npm, PyPI, and Homebrew.

## Documenting a change (required for every PR that touches a package)

Add a change file under `.changeset/` describing which packages your PR
changes and how their versions should bump:

```markdown
---
roder-core: minor
roder: patch
---

# Short summary of the change

Optional longer description in Markdown.
```

- File name: anything ending in `.md`, e.g. `.changeset/fix_cheese_distribution.md`.
- Package names are the keys in `knope.toml` (crate names for Rust;
  `roder-sdk-typescript`, `roder-sdk-python`, `roder-edit-tools` for the SDKs).
- Bump types: `major`, `minor`, or `patch`.
- The summary becomes the changelog / release-notes entry.
- Interactive alternative: install knope and run `knope document-change`.
- Do **not** put non-changeset files (e.g. a README) in `.changeset/` — knope
  treats every `.md` file there as a change file.

## Versioning gate (CI)

`.github/workflows/changeset-gate.yml` fails a PR when:

- a file under a released package's directory changed but no changeset added
  in the PR names that package;
- any pending changeset is malformed, names an unknown package, or uses an
  invalid bump type;
- `knope.toml` is stale (see "Adding a package").

Bypasses:

- Apply the `skip-changeset` PR label for changes that shouldn't be released
  (e.g. test-only or comment-only changes inside a crate).
- Changes outside package directories (docs, scripts, workflows, roadmap)
  never require a changeset.
- The knope release PR (branch `knope/release`) is exempt because it deletes
  changesets by design.

## Release Flow

1. PRs merge to `master`, each carrying changesets.
2. `prepare-release.yml` runs on every push to `master`: knope combines all
   pending changesets, bumps each affected package's version (crate
   `Cargo.toml` + `Cargo.lock`, `package.json` + `package-lock.json`,
   `pyproject.toml`), writes per-package `CHANGELOG.md` entries, deletes the
   consumed changesets, and force-pushes the `knope/release` branch with an
   open release preview PR.
3. Merging that release PR is the release gate. `release.yml` then runs
   `knope release`, tagging each released package (`<name>/v<version>`) and
   creating a GitHub release per package with its release notes.
4. Registry publication is manual today. Before publishing, run the registry
   README gate:

   ```sh
   make registry-readmes
   ```

Unchanged packages are untouched: no version bump, no tag, no release.

## Configuration

- `knope.toml` is **generated** — never edit it by hand. Regenerate with
  `python3 scripts/generate-knope-config.py` (CI verifies with `--check`).
- The generator scans `crates/*/Cargo.toml` and adds the SDK packages listed
  in its `EXTRA_PACKAGES` table.
- `scripts/check-changesets.py` implements the CI gate; it derives package
  directories from `knope.toml`.

## Adding a package

1. Create the crate with an explicit `version = "0.1.0"` in its `Cargo.toml`
   (crates do **not** use `version.workspace = true`; the workspace has no
   shared version).
2. Run `python3 scripts/generate-knope-config.py` and commit `knope.toml`.
3. For non-Rust packages, add an entry to `EXTRA_PACKAGES` in
   `scripts/generate-knope-config.py` first.
4. Optionally run the "Baseline release tags" workflow (or
   `python3 scripts/init-release-tags.py --push`) to tag the package's
   current version so it isn't released before its first real change.

## One-time setup after adopting knope

Run the **Baseline release tags** workflow (Actions → "Baseline release
tags" → Run workflow). It tags every package at its current version so the
first `knope release` only releases packages with real changes.

## GitHub setup

GitHub Actions must be allowed to create pull requests: Settings → Actions →
General → Workflow permissions → Read and write permissions, with pull
request creation enabled. The workflows only use `GITHUB_TOKEN`.

Note: because the release preview PR is created with `GITHUB_TOKEN`, other
workflows (CI) don't run on it. If you later make the changeset gate or other
checks required for merging, switch `prepare-release.yml` to a PAT.

## Registry Publication

Before any registry publish:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```

README requirements:

- Every Cargo crate must have its own package-local `README.md` and
  `readme = "README.md"` in `Cargo.toml`.
- npm packages must include `README.md` in `package.json` `files`.
- PyPI packages must keep `[project] readme = "README.md"` in `pyproject.toml`.
- Every package README must link to `https://roder.sh`.

Cargo packages publish in dependency order and crates.io may rate-limit new
crate creation. Use `cargo metadata` to derive the workspace order and include
dev-dependencies in the ordering because `cargo publish` verifies package
tarballs with dev-dependencies resolvable from the registry.

For npm:

```sh
cd sdk/typescript
pnpm pack --dry-run
npm publish --access public --registry=https://registry.npmjs.org/

cd ../../packages/edit-tools
pnpm pack --dry-run
npm publish --access public --registry=https://registry.npmjs.org/
```

For PyPI:

```sh
cd sdk/python
uv build
uv publish
```

For Homebrew and macOS binary signing, CI updates the tap and release assets
automatically — see the "Homebrew" and "macOS signing" sections below. Manual
steps are only needed for recovery.

## Registry Publication Readiness

Before publishing crates to crates.io later:

1. Add publish metadata (description, license, repository) to public crates.
2. Add registry versions to internal workspace dependencies, for example
   `roder-api = { path = "crates/roder-api", version = "0.1.0" }`, and
   extend the knope config generator to keep those dependency versions
   updated (`{ path = "Cargo.toml", dependency = "roder-api" }`).
3. Mark private crates with `publish = false`.
4. Add a publish step to `release.yml` gated on a `CARGO_REGISTRY_TOKEN`.

## Homebrew

`brew install RoderAI/tap/roder` is kept current automatically and installs the
signed Apple Silicon release archive by default. Users who want to build locally
can run `brew install --with-source RoderAI/tap/roder`.

When the release PR (`knope/release`) merges, `release.yml` runs `knope release`
to tag `roder/v<version>`, builds release archives, signs and notarizes the
`aarch64-apple-darwin` archive, uploads those archives to the `roder/v<version>`
GitHub release, then runs `scripts/update-homebrew-tap.sh`, which:

1. resolves the released `roder` version from `crates/roder-cli/Cargo.toml`;
2. downloads the signed Apple Silicon release archive
   (`roder-aarch64-apple-darwin.tar.gz`) and immutable source tag tarball, then
   computes their `sha256` values;
3. regenerates `Formula/roder.rb` in `RoderAI/homebrew-tap` so default installs
   use the binary archive and `--with-source` uses the source tag;
4. commits and pushes it (no-op when the tap is already on that version).

The `homebrew` job lives in the same workflow run as `knope release` on purpose:
a tag/release created with the default `GITHUB_TOKEN` does not trigger a separate
downstream workflow, so the tap update has to run inline.

### Required configuration

- Repository secret **`HOMEBREW_TAP_TOKEN`**: a PAT (or fine-grained token) with
  `contents: write` on `RoderAI/homebrew-tap`. Without it the job fails loudly;
  the crate tags/releases from the earlier step are unaffected.
- Repository secret **`APPLE_CERTIFICATE_BASE64`**: base64-encoded Developer ID
  Application `.p12` certificate for signing the macOS CLI.
- Repository secret **`APPLE_CERTIFICATE_PASSWORD`**: password for the `.p12`.
- Repository secret **`APPLE_NOTARIZE_KEY_BASE64`**: base64-encoded App Store
  Connect API key `.p8` for notarization.
- Repository secret **`APPLE_NOTARIZE_KEY_ID`**: App Store Connect API key ID.
- Repository secret **`APPLE_NOTARIZE_ISSUER_ID`**: App Store Connect issuer ID.

### Manual run / recovery

Run the same automation locally (dry-run writes the formula under `dist/`
without pushing):

```sh
# Inspect the rendered formula only:
VERSION=0.1.5 make update-homebrew-tap

# Actually push to the tap:
VERSION=0.1.5 HOMEBREW_TAP_TOKEN=ghp_... ./scripts/update-homebrew-tap.sh
```

Then validate against the tap:

```sh
brew audit --strict --online --new RoderAI/tap/roder
brew reinstall RoderAI/tap/roder
brew reinstall --with-source RoderAI/tap/roder
brew test RoderAI/tap/roder
```

`scripts/release-brew.sh` (`make release-brew`) remains a separate, fully manual
helper for cutting a local source release; it is not part of the automated flow.

## macOS signing

Roder publishes signed, notarized macOS binaries for Apple Silicon only. The
`publish-latest-roder.yml` workflow signs the latest `aarch64-apple-darwin`
binary before uploading it to the `latest` GitHub release; the release workflow
signs the same target before uploading versioned release archives to GitHub.

The signing path uses Anchore Quill with a Developer ID Application certificate
and App Store Connect API key. Intel macOS release binaries are intentionally not
published; `--with-source` remains available for users who explicitly want a
local build.
