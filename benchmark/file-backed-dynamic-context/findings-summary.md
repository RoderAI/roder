# File-Backed Dynamic Context Findings Summary

## Headline

- Fixtures run: `2`
- Hidden-detail recovery: `2/2`
- Inline tokens before: `71652`
- Inline tokens after: `226`
- Inline tokens saved: `71426`
- Artifact bytes written: `286608`
- Artifact lines written: `3300`
- Artifact grep calls: `2`
- Total benchmark wall time: `7 ms`

## Findings

- File-backed context recovered every hidden detail in the current offline fixture set.
- The long-command fixture shows the intended win: most log bytes move out of inline context while a single artifact grep recovers the token.
- The compaction-history fixture confirms the summary can remain compact while exact prior details stay recoverable through a chat-history artifact.

## Current Limitations

- This benchmark uses deterministic offline fixture payloads and local artifact operations, not live provider turns.
- Runtime ablation is available with `[context].file_backed_dynamic_context = false` or `RODER_DISABLE_CONTEXT_ARTIFACTS=1`; this offline benchmark has not yet generated a side-by-side ablation table.
