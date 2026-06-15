---
roder-supergrok-auth: patch
---

# Improve SuperGrok xAI OAuth 2.0 device authorization flow

- Fix unit test compilation after adding device_authorization_endpoint support (struct literals in tests).
- Return the signed-in email (parsed from id_token when present) from `device_flow()` so `roder auth login supergrok` prints the account email on success, matching behavior of status and Codex login.
- Use a reqwest client with a custom User-Agent ("Roder/1.0 (+https://roder.sh)") for all discovery, device authorization, token exchange, and refresh calls. This improves reliability against Cloudflare protections on auth.x.ai endpoints.
- All device flow steps (client_id, scope, grant_type device_code, polling for authorization_pending/slow_down/expired) continue to match xAI's published OIDC discovery and supported grant types.
