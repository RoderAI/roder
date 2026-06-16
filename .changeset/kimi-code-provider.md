---
"roder-ext-kimi-code": minor
"roder-cli": patch
"roder-api": patch
"roder-app-server": patch
"roder-protocol": patch
"roder-extension-host": patch
---

Added first-class `kimi-code` (aliases: `kimi`, `moonshot`) inference provider and `roder-ext-kimi-code` crate.

- Direct Moonshot API (`api.moonshot.ai/v1`) OpenAI-compatible streaming + tools.
- Catalog entry + `kimi-k2.6` model (plus aliasing in normalize rules).
- Device OAuth against `auth.kimi.com` with `roder auth login kimi-code`, TUI/app-server `auth/kimi-code/*`, and token storage under `~/.roder/auth/kimi-code.json`.
- API key fallback via env/config (`KIMI_CODE_API_KEY`, `RODER_KIMI_CODE_API_KEY`).
- Registered via extension host (always available, like SuperGrok).
- Docs: `docs/roder-kimi-code-provider.md`.
- Live smoke test added (opt-in via `RODER_KIMI_CODE_LIVE=1`).
- No changes to OpenCode Go paths (Kimi models remain available under both routes).