---
name: vcs-snapshot
description: Create scoped VCS snapshots from the current workspace state.
metadata:
  short-description: Snapshot safely
exposure: direct_only
---
Inspect VCS status and the relevant diff before creating a provider history snapshot.

Select only files or hunks that the user asked to include, if the active provider supports that granularity. Ignore unrelated changes from other agents unless the user explicitly asks to include them.

Write a concise snapshot message that reflects the saved behavior. In git workspaces this maps to a commit, so report the commit hash; for other providers, report the provider snapshot identity and the validation evidence used for the slice.
