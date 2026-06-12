---
name: create-changeset
description: Use when a PR or branch changes any released package (crates/*, sdk/typescript, sdk/python, packages/edit-tools), when the changeset-gate CI check fails, or before pushing work that touches versioned package directories. Creates the required .changeset/*.md file for knope per-package versioning.
metadata:
  short-description: Add knope changesets for changed packages
---

# Create a changeset

Roder uses [knope](https://knope.tech) with **per-package changesets**. Commit
messages never bump versions — only files in `.changeset/` do. CI enforces this
via `.github/workflows/changeset-gate.yml`.

Broader release context: `docs/releases.md` and `$changesets`.

## When you need a changeset

Add one when your branch modifies files under any **released package directory**:

| Path pattern | knope package name |
| --- | --- |
| `crates/<crate>/` | crate name from `Cargo.toml` (usually `roder-*`) |
| `sdk/typescript/` | `roder-sdk-typescript` |
| `sdk/python/` | `roder-sdk-python` |
| `packages/edit-tools/` | `roder-edit-tools` |

**No changeset needed** for docs-only, roadmap, scripts, workflows, or
examples that are not under a released package root.

## Workflow (follow in order)

### 1. See what CI will complain about

From repo root, against the PR base branch (usually `master`):

```sh
python3 scripts/check-changesets.py --base origin/master
```

If it fails, the error lists missing package names and prints a template.

Also confirm knope config is current (second CI step):

```sh
python3 scripts/generate-knope-config.py --check
```

If stale, run `python3 scripts/generate-knope-config.py` and commit `knope.toml`
(see `$changesets` when adding a new crate).

### 2. List packages your branch actually touched

```sh
git fetch origin master
git diff --name-only "$(git merge-base origin/master HEAD)" HEAD
```

Map each changed path to a package: anything under `crates/roder-api/` →
`roder-api`, under `crates/roder-ext-process-host/` → `roder-ext-process-host`,
etc. Package names are the keys in `knope.toml` — when unsure, grep:

```sh
rg '^\[packages\."' knope.toml | rg roder-api
```

One changeset file can cover **multiple packages** changed by the same PR.

### 3. Choose bump types

Frontmatter bump per package: `major`, `minor`, or `patch`.

| Change | Typical bump (pre-1.0) |
| --- | --- |
| Bug fix, internal refactor, tests, docs inside package | `patch` |
| New feature, new public API, new protocol surface | `minor` |
| Breaking public API or wire contract | `minor` (pre-1.0) or `major` (post-1.0) |

When in doubt for additive work, use `patch`. Use `minor` when clients or
integrators must notice new capability.

### 4. Write the change file

Create **one** descriptive file, e.g. `.changeset/cursor_sdk_process_extension.md`:

```markdown
---
roder-api: patch
roder-app-server: patch
roder-ext-process-host: patch
---

# Short summary (becomes changelog heading)

Optional longer Markdown body for release notes.
```

Rules:

- File **must** start with `---` frontmatter and a closing `---` before the body.
- Only `.md` files belong in `.changeset/` — no READMEs or notes there.
- Name packages exactly as in `knope.toml` (quoted keys in TOML, unquoted in YAML).
- Summary line after frontmatter should describe the **user-visible** change, not
  the commit message.

Interactive alternative (if knope is installed): `knope document-change`.

### 5. Verify and commit

**The changeset must be committed.** CI diffs `merge-base..HEAD`; an untracked
file does not satisfy the gate until it is in the branch history.

```sh
python3 scripts/check-changesets.py --base origin/master
git add .changeset/your_file.md
git commit -m "chore: add changeset for <short description>"
```

Expected success output:

```text
Changeset gate passed: roder-api, roder-app-server covered.
```

### 6. Push

Push the commit with the changeset before expecting CI green (account billing
must also allow Actions to run).

## Bypass (rare)

Apply the PR label `skip-changeset` only for changes that truly should not
release (e.g. comment-only or test-only edits inside a crate with no behavioral
change worth noting). Do not use the label to avoid documenting real features.

The `knope/release` branch is exempt from the gate by design.

## Common mistakes

| Mistake | Fix |
| --- | --- |
| Created `.changeset/foo.md` but did not commit it | `git add` + commit, re-run check |
| Wrong package name (`roder_ext_process_host`) | Use knope key: `roder-ext-process-host` |
| Put README in `.changeset/` | Move it elsewhere; knope parses every `.md` there |
| Edited `knope.toml` by hand | Regenerate with `scripts/generate-knope-config.py` |
| Changed a new crate but gate says unknown package | Regenerate `knope.toml` after adding the crate |

## Example (this repo)

`.changeset/cursor_sdk_process_extension.md`:

```markdown
---
roder-api: patch
roder-app-server: patch
roder-ext-process-host: patch
---

# Process-extension protocol 0.2.0 and Cursor SDK remote-agent bridging

Extend the process-extension protocol with subagent-dispatcher and task-executor
services, bridge them in the process host, and add app-server e2e coverage for
the cursor-sdk-agents TypeScript child.
```
