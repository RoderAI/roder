# Roder App-Server API Documentation Standard

Use this standard when creating or maintaining `docs/roder-app-server-api.md`.

## Target Reader

Write for an API integrator building a client against the Roder app-server. Assume they understand HTTP, JSON-RPC, streaming events, and agent runtimes, but do not know the Roder source tree.

## Required Document Shape

`docs/roder-app-server-api.md` should include these sections when applicable:

1. Overview
2. Transport and base URL assumptions
3. Authentication and local credential expectations
4. Core concepts: sessions, threads, turns, providers, models, extensions
5. Method or endpoint index
6. Detailed method reference
7. Streaming/event reference
8. Error model and cancellation semantics
9. Persistence and compatibility notes
10. Integration recipes
11. Maintenance checklist

Keep the method index compact. Put detailed examples in the method reference.

## Method Reference Template

Use this shape for each method or endpoint:

```markdown
### `method/name`

Purpose: One sentence explaining why a client calls this.

Request:
```json
{
  "example": "value"
}
```

Response:
```json
{
  "example": "value"
}
```

Behavior:
- Describe lifecycle effects, persistence, streaming, default values, and ordering constraints.

Errors:
- List known validation, not-found, auth, provider, cancellation, and runtime error behavior.

Notes:
- Include compatibility or experimental details only when useful.
```

## Streaming/Event Reference Template

For every client-visible event, document:

- Event name or discriminator.
- When it is emitted.
- Payload fields and required/optional status.
- Ordering guarantees relative to requests or other events.
- Retry, cancellation, or terminal-state behavior.

Use valid JSON examples for event payloads.

## Integration Recipes

Prefer short, end-to-end flows over scattered notes. Useful recipes include:

- Create or resume a session.
- Start a turn and consume events until terminal state.
- Interrupt or cancel an active turn.
- List/select providers and models.
- Persist and retrieve thread-scoped extension state.
- Poll or bridge app-server behavior from a desktop client.

## Accuracy Rules

- Every field in examples must be backed by source, tests, or existing docs.
- Do not document internal-only structs unless a client sees them on the wire.
- Use consistent names for `session`, `thread`, `turn`, `provider`, `model`, and `extension` concepts.
- Mark removed or replaced behavior as deprecated only when the source still supports it.
- If source and existing docs disagree, update the docs to source truth and mention the discrepancy in the final summary.

## Maintenance Checklist

When updating `docs/roder-app-server-api.md`, check whether the change affects:

- Method index entries.
- Request/response examples.
- Event examples.
- Error handling text.
- Auth and environment variable requirements.
- Provider/model notation and defaults.
- Session/thread persistence notes.
- Desktop or sibling-client integration recipes.
