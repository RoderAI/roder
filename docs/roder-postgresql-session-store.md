# Roder PostgreSQL Session Store

PostgreSQL session storage is an opt-in replacement for the default local JSONL thread store. It persists thread metadata, raw runtime events, public thread item events, extension state, and context artifacts in tenant-scoped PostgreSQL tables.

## Configuration

JSONL remains the default. Select PostgreSQL with config:

```toml
[sessions]
store = "postgres"

[sessions.postgres]
database_url_env = "RODER_POSTGRES_SESSION_URL"
tenant_id_env = "RODER_POSTGRES_SESSION_TENANT"
max_connections = 5
```

or with environment variables:

```sh
export RODER_SESSION_STORE=postgres
export RODER_POSTGRES_SESSION_URL='postgres://roder:roder@localhost:5432/roder'
export RODER_POSTGRES_SESSION_TENANT='tenant-a'
roder
```

The tenant id is trusted configuration or auth context. It is not read from prompts, model tool arguments, or arbitrary app-server request payloads.

## Local PostgreSQL

```sh
docker run --rm --name roder-postgres \
  -e POSTGRES_USER=roder \
  -e POSTGRES_PASSWORD=roder \
  -e POSTGRES_DB=roder \
  -p 5432:5432 postgres:16
```

The extension runs idempotent `CREATE TABLE IF NOT EXISTS` migrations on startup and records the migration version in `roder_session_migrations`.

## Context artifacts

When PostgreSQL is active, context artifacts are stored in `roder_context_artifacts` under the same `(tenant_id, thread_id)` scope as sessions. The runtime asks the active thread store for its artifact backend, so PostgreSQL mode does not silently fall back to `~/.roder/context-artifacts`.

Artifact descriptors returned to clients omit database URLs and raw SQL details.

## Verification

```sh
cargo test -p roder-ext-postgres-session
RODER_POSTGRES_SESSION_TEST_URL=postgres://roder:roder@localhost:5432/roder_test \
  cargo test -p roder-ext-postgres-session --test postgres_session -- --ignored
cargo test -p roder-config sessions
```

## Troubleshooting

- `database URL is required`: set `RODER_POSTGRES_SESSION_URL` or `[sessions.postgres].database_url(_env)`.
- `tenant id is required`: set `RODER_POSTGRES_SESSION_TENANT` or `[sessions.postgres].tenant_id(_env)`.
- Connection refused: verify PostgreSQL is running and reachable from the Roder process.
- Migration permission failure: grant `CREATE TABLE`, `INSERT`, `UPDATE`, `SELECT`, and `DELETE` on the configured database/schema.
- Credential leaks: Roder redacts password-bearing URLs in PostgreSQL session configuration errors.

## Backup and restore

Back up the database using your normal PostgreSQL tooling, for example `pg_dump`. Restore into a database with the same tenant ids if sessions must remain visible to existing deployments.
