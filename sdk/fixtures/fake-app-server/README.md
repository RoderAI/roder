# Fake App-Server Fixtures

These JSONL files use the `roder-api-transcript` schema shape: `header`, `api.request`, `api.response`, and `api.notification` records.

They are synthetic and deterministic. Paths, ids, timestamps, providers, and model names are fixed test values. SDK tests in TypeScript and Python replay the same files through in-memory transports so normal checks do not require a real app-server, network, provider credentials, plugins, or user config.

`workspace-files-flow.jsonl` covers the workspace file API used by file trees, quick-open, and mentions: status, rebuild, status-change notifications, root children, directory children, ranked query, and preview read.
