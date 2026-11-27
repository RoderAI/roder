---
roder-api: patch
roder-app-server: patch
roder: patch
roder-code-index: patch
roder-config: patch
roder-core: patch
roder-ext-anthropic: patch
roder-ext-chrome: patch
roder-ext-codex-backend: patch
roder-ext-cursor: patch
roder-extension-host: patch
roder-ext-gemini: patch
roder-ext-git: patch
roder-ext-jev: patch
roder-ext-openai-chat-completions: patch
roder-ext-openai-responses: patch
roder-ext-parallel-search: patch
roder-ext-runner-sprites: patch
roder-ext-subagents: patch
roder-ext-task-process: patch
roder-protocol: patch
roder-supergrok-auth: patch
roder-tools: patch
roder-tui: patch
---

Fix every `cargo clippy --workspace --all-targets -D warnings` error. The fixes are behavior-preserving lint cleanups (collapsed `if let` chains, removed no-op conversions, derived `Default`s, test modules moved after the items they test). Targeted `#[allow]`s with reasons cover wire types and const table constructors.
