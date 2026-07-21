## Unreleased

### Fixes

#### Coalesce parallel tool calls for longer OpenCode/DeepSeek rollouts

OpenCode chat-completions requests now coalesce consecutive parallel tool calls
into a single assistant `tool_calls` message and immediately pair each id with a
`role: tool` result (or a placeholder). This matches the OpenAI-compatible
invariant required by DeepSeek-backed OpenCode gateways and prevents
`400 invalid_request_error` failures on multi-step tool turns.

#### Clearer OpenCode provider error messages

Chat Completions failures now surface structured OpenCode error details such as
`ModelError: Model is disabled` and `CreditsError: No payment method` instead of
only the raw status/body blob.

## 0.1.2 (2026-07-21)

### Fixes

#### Fix OpenCode DeepSeek multi-step tool rollouts

Refresh the OpenCode Zen model catalog (drop disabled free DeepSeek IDs, add
current free models and paid `deepseek-v4-flash` / `deepseek-v4-pro`), coalesce
parallel tool calls into valid chat-completions histories for longer DeepSeek
rollouts, and surface clearer OpenCode ModelError/CreditsError messages.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
