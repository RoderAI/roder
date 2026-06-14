---
name: release-notes
description: Draft release notes for Roder releases, new crates or packages, feature additions, behavior changes, and fixes. Use when writing release notes, changelog summaries, release announcements, or explaining what changed in a branch, PR, crate, SDK, or package release.
metadata:
  short-description: Draft Roder release notes
---

# Release Notes

Use this skill to turn code, changesets, PRs, or changelog entries into clear
release notes that explain what was added, what changed, and what was fixed.

## Workflow

1. Identify the release scope:
   - Repo-wide release, single crate, SDK, package, PR, or branch.
   - Target audience: users, integrators, contributors, or maintainers.
   - Source range: tags, commits, PR diff, `.changeset/*.md`, or existing
     `CHANGELOG.md` entries.

2. Gather evidence before drafting:
   - Read relevant `.changeset/*.md` files first when present.
   - Check changed package names in `knope.toml` or package manifests when scope
     is unclear.
   - Inspect diffs or changelogs for user-visible behavior, APIs, docs, tests,
     and bug fixes.
   - Ignore unrelated dirty-tree work unless it is in the requested release
     scope.

3. Categorize changes:
   - **Added**: New crates, packages, commands, APIs, provider support, UI
     surfaces, docs, or workflows.
   - **Changed**: Behavior differences, renamed concepts, new defaults,
     protocol shape changes, UX refinements, dependency/runtime changes, or
     migration-relevant differences.
   - **Fixed**: Bugs, crashes, incorrect output, flaky tests, race conditions,
     broken docs, compatibility issues, or release automation failures.
   - **Removed**: Deleted APIs, commands, config, docs, packages, or behavior.
     Include only when relevant.

4. Write from the user's point of view:
   - Prefer concrete outcomes over implementation details.
   - Mention affected crates/packages when helpful.
   - Call out breaking changes or required user action plainly.
   - Do not invent impact. If a change is internal-only, label it as internal or
     omit it from user-facing release notes.
   - Keep each bullet short and specific.

## Output Template

Use this shape by default, trimming empty sections:

```markdown
## Release Notes

### Added
- Added ...

### Changed
- Changed ...

### Fixed
- Fixed ...

### Removed
- Removed ...

### Notes
- Breaking change: ...
- Migration: ...
- Internal: ...
```

For a single package release, include the package name and version if known:

```markdown
## `roder-example` v0.1.0

### Added
- Added ...
```

## Roder-Specific Notes

- New crates use `version = "0.1.0"` and require regenerated `knope.toml`.
- Released package changes should have `.changeset/*.md` coverage. Use
  `$create-changeset` when a release note reveals a missing changeset.
- This project moves quickly; describe the current canonical behavior instead
  of documenting legacy shims unless the release actually ships a compatibility
  path.
