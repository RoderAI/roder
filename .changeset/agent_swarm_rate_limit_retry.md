---
roder-api: minor
roder-config: minor
roder-ext-subagents: minor
---

# Rate-limit-aware agent_swarm scheduling

Swarm children that fail with a provider rate limit are now requeued with
exponential backoff (default 3s, 6s, 12s, ... up to 4 retries) instead of
failing outright (roadmap 104, Task 3). The concurrency permit is held across
the backoff so a rate-limited swarm naturally throttles rather than hammering
the provider, and cancellation still wins promptly. Tunable via
`[agent_swarm].rate_limit_max_retries` / `rate_limit_base_backoff_ms` and the
`RODER_AGENT_SWARM_RATE_LIMIT_*` env vars (retries are clamped to a hard cap so
a swarm can never wait unboundedly).
