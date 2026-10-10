---
roder-api: minor
roder-codex-auth: minor
roder-ext-jev: patch
roder-extension-host: patch
roder-ext-xai: patch
roder-ext-kimi-code: patch
roder-ext-openai-responses: patch
---

# Name the program embedding Roder in sign-in hints

Programs that embed the Roder runtime ship their own command line, so a hint
such as "run `roder auth login codex`" means nothing to their users.
`roder_api::cli_identity` lets the embedding program set its identity once at
startup with `set_cli_identity(CliIdentity { name, auth_login, auth_dir })`.

- Every sign-in hint (Codex, SuperGrok, Kimi Code, and Jev's text-model notes)
  now names the configured `auth_login` command, with `{provider}` replaced by
  the provider id. Roder's own default is `roder auth login {provider}`.
- `roder_codex_auth::Store::new()` keeps tokens in the configured `auth_dir`,
  falling back to Roder's data folder (`RODER_DATA_DIR`, else `~/.roder`).
- The Codex sign-in's busy-port error names the configured program.
