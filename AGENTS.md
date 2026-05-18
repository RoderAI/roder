If you see work that you don't recognise: IGNORE it. There are multiple agents working on the repo at any one time.

When writing files start to split logic out form files that are 500+ lines long into smaller componetised logic. Dont over split.

This is a brand new project moving quickly. Do not add backwards-compatibility shims, legacy aliases, migration paths, or deprecated duplicate APIs for surfaces that are meant to move forward. Prefer updating callers and docs to the new canonical API, even when that is a breaking change.
