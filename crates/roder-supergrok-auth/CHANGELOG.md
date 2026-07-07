## 0.1.3 (2026-07-07)

### Fixes

#### Make provider browser auth robust on WSL and add Kimi Code API key login

Print auth URLs before opening browsers, fall back to WSL-friendly browser commands, and allow `roder auth login kimi-code --api-key [KEY]`.

## 0.1.2 (2026-06-22)

### Fixes

- Stabilize Roder startup, streaming responses, and provider behavior

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.

#### Improve SuperGrok xAI OAuth 2.0 device authorization flow

- Fix unit test compilation after adding device_authorization_endpoint support (struct literals in tests).
- Return the signed-in email (parsed from id_token when present) from `device_flow()` so `roder auth login supergrok` prints the account email on success, matching behavior of status and Codex login.
- Use a reqwest client with a custom User-Agent ("Roder/1.0 (+https://roder.sh)") for all discovery, device authorization, token exchange, and refresh calls. This improves reliability against Cloudflare protections on auth.x.ai endpoints.
- All device flow steps (client_id, scope, grant_type device_code, polling for authorization_pending/slow_down/expired) continue to match xAI's published OIDC discovery and supported grant types.
