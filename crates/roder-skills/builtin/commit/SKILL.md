---
name: commit
description: Create scoped git commits from the current repository state.
metadata:
  short-description: Commit safely
exposure: direct_only
---
Inspect git status and the relevant diff before committing.

Stage only files or hunks that the user asked to include. Ignore unrelated changes from other agents unless the user explicitly asks to include them.

Write a concise commit message that reflects the committed behavior. After committing, report the commit hash and the validation evidence used for the slice.
