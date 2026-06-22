# Changelog
## 0.1.1 (2026-06-22)

### Features

#### Add first-party Synthetic inference provider

Adds the `synthetic` provider using Synthetic's OpenAI-compatible Chat
Completions API. The provider ships built-in `syn:` model aliases
(`syn:large:text` default, plus `syn:small:text`, `syn:large:vision`,
`syn:small:vision`), preserves concrete `hf:{owner}/{model}` ids across config,
discovery, and selection, and resolves credentials only from
`SYNTHETIC_API_KEY`/`RODER_SYNTHETIC_API_KEY` or `[providers.synthetic]`. The
provider is visible without credentials so app-server and TUI can show setup
state, and turn-time inference fails locally with setup guidance when the key
is missing. The TUI provider menu points to the Synthetic dashboard for API-key
setup instead of the generic fallback URL.

### Fixes

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
