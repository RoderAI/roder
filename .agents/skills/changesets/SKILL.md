---
name: changesets
description: Use when a PR changes any released package (crates/*, sdk/typescript, sdk/python, packages/edit-tools), when the changeset-gate CI check fails, when adding a new crate or package to the release config, or when working on knope versioning/release automation.
---

# Changesets & per-package versioning (knope)

This repo versions every Cargo crate and SDK package independently using
[knope](https://knope.tech) with changesets. Commit messages never bump
versions; only `.changeset/*.md` files do. Full docs: `docs/releases.md`.

## Add a changeset (required by CI)

Any PR touching files under a released package's directory must add a change
file in `.changeset/`:

```markdown
---
roder-core: minor
roder-cli: patch
---

# Short summary used in changelogs and release notes
```

- One file per logical change; name it descriptively, e.g.
  `.changeset/add_mcp_streaming.md`.
- Package names = keys in `knope.toml` (crate names; SDKs are
  `roder-sdk-typescript`, `roder-sdk-python`, `roder-edit-tools`).
- Bump types: `major` | `minor` | `patch`. Pre-1.0, breaking changes are
  usually `minor`, everything else `patch`.
- Never put non-changeset `.md` files (READMEs etc.) in `.changeset/`.

Verify locally before pushing:

```sh
python3 scripts/check-changesets.py --base origin/master
```

Test-only or comment-only changes inside a package can bypass the gate with
the `skip-changeset` PR label instead of a changeset.

## Adding a crate or package

1. New crates get an explicit `version = "0.1.0"` in `Cargo.toml` — the
   workspace has no shared version, do not use `version.workspace = true`.
2. Regenerate the release config (CI checks it's current):

   ```sh
   python3 scripts/generate-knope-config.py
   ```

3. Non-Rust packages: add to `EXTRA_PACKAGES` in
   `scripts/generate-knope-config.py` first.
4. `knope.toml` is generated — never edit it by hand.

## How releases happen (context)

- Push to `master` → `prepare-release.yml` runs `knope prepare-release`,
  which consumes pending changesets, bumps affected packages, updates their
  `CHANGELOG.md`s, and maintains the `knope/release` preview PR.
- Merging that PR → `release.yml` runs `knope release`, tagging
  `<package>/v<version>` and creating one GitHub release per package.
- Releases are git-only (no crates.io / npm / PyPI publication).
- Baseline tags for unreleased packages: `python3 scripts/init-release-tags.py`
  or the "Baseline release tags" workflow.
