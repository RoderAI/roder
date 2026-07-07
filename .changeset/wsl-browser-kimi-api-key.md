---
roder: patch
roder-app-server: patch
roder-codex-auth: patch
roder-ext-kimi-code: patch
roder-supergrok-auth: patch
roder-tui: patch
---

# Make provider browser auth robust on WSL and add Kimi Code API key login

Print auth URLs before opening browsers, fall back to WSL-friendly browser commands, and allow `roder auth login kimi-code --api-key [KEY]`.
