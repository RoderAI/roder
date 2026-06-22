## 0.1.2 (2026-06-22)

### Fixes

- Stabilize Roder startup, streaming responses, and provider behavior

#### Kimi Code OAuth chat requests omit unsupported OpenAI-compat fields

OAuth turns on `api.kimi.com/coding/v1` no longer send `stream_options` or
`parallel_tool_calls`, which caused 400 responses on the managed Kimi Code API.
Adds configurable flags on the shared chat-completions helper and gates
`should_compact_transcript` to test builds only.

#### Show full provider error response body in the timeline detail popup

Chat Completions provider errors (e.g. Synthetic 502 Bad Gateway) previously
discarded the response body entirely and showed only a generic hint like
"provider server error; response body redacted". Now the error includes the
full response body after a separator, so clicking the error row (or pressing
Enter on it) opens the existing tool-detail popup with the complete provider
response text — the same popup used for shell output and file edits.

The auth credential (bearer token or api-key header value) is scrubbed from
the body before it enters the error message, so the popup is safe to share.
Bodies are capped at 4 KB to avoid unbounded error messages.

The TUI also makes `Error` timeline items selectable and clickable, routes
Enter on a selected error row to the detail modal, and renders the popup with
an "error details" title and "Response body" label when the source is a
provider error rather than a shell command.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
