---
roder-api: minor
roder-config: minor
roder-ext-subagents: patch
---

# Global agent-swarm rate-limit capacity governor

Add the global capacity-shrink / quiet-window recovery throttle for the
`agent_swarm` scheduler (roadmap 104, Task 3 follow-up), so a swarm backs off as
a whole under sustained provider rate limits instead of every child retrying in
parallel with only per-child backoff.

A shared `RateLimitGovernor` is inert until the first provider rate limit. On the
first rate limit it sizes a global capacity from the children that were active
when it hit, then shrinks by one; later rate limits shrink by one more (floor of
one) no more often than `rate_limit_shrink_interval_ms` (default 2000), and
launches are paced apart while throttled. After a quiet
`rate_limit_recovery_interval_ms` (default 180000, three minutes) with no rate
limit, the swarm recovers one unit of capacity. The normal-phase ramp, overlap,
ordering, and `max_concurrency` cap are unchanged.

New bounded config knobs (`[agent_swarm].rate_limit_shrink_interval_ms`,
`rate_limit_recovery_interval_ms`) and matching
`RODER_AGENT_SWARM_RATE_LIMIT_SHRINK_INTERVAL_MS` /
`RODER_AGENT_SWARM_RATE_LIMIT_RECOVERY_INTERVAL_MS` env overrides resolve the
windows. Covered by fake-clock (`tokio::time::pause`) tests for shrink, recovery,
and a sustained-rate-limit end-to-end run that completes in order without
deadlock.
