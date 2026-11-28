# Roadmapping Mode Implementation Plan

**Goal:** Add a document-first roadmapping mode that plans and tracks work in checked-in Markdown.
**Architecture:** The runtime reads roadmap Markdown documents, derives stable task ids from their headings, and records progress in a JSON state file beside them.
**Tech Stack:** Rust, Markdown, JSON.

## Owned Paths

- Create: `crates/roder-roadmap/src/lib.rs`
- Modify: `roadmap/README.md`

## Tasks

### Task 1: Parser

- [x] Add tests that parse existing roadmap files

Run:

```sh
cargo test -p roder-roadmap --test roadmap
```

Acceptance:
- Titles, goals, owned paths, tasks and acceptance items are parsed.

### Task 2: Checkbox updates

- [ ] Update a task checkbox without touching other lines

Run:

```sh
cargo test -p roder-roadmap checkbox
```

Acceptance:
- Every other byte of the document is preserved.

## Phase Acceptance

- [ ] Normal thread sessions are unchanged when roadmapping mode is off.
