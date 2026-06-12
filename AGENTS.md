If you see work that you don't recognise: IGNORE it. There are multiple agents working on the repo at any one time.

When writing files start to split logic out form files that are 500+ lines long into smaller componetised logic. Dont over split.

This is a brand new project moving quickly. Do not add backwards-compatibility shims, legacy aliases, migration paths, or deprecated duplicate APIs for surfaces that are meant to move forward. Prefer updating callers and docs to the new canonical API, even when that is a breaking change.

Versioning: every PR that changes a released package (any `crates/*` crate, `sdk/typescript`, `sdk/python`, `packages/edit-tools`) must add a changeset file in `.changeset/` naming the affected packages and bump types (`major`/`minor`/`patch`) — CI enforces this. New crates use an explicit `version = "0.1.0"` (not `version.workspace = true`) and require regenerating `knope.toml` via `python3 scripts/generate-knope-config.py`. See `docs/releases.md`.
