---
roder-api: minor
roder-app-server: minor
roder: patch
roder-core: minor
roder-ext-jsonl-thread-store: patch
roder-protocol: minor
roder-tui: patch
roder-sdk-typescript: minor
roder-sdk-python: minor
---

# Set inference service tiers per thread or turn

Accept `serviceTier` on `thread/start` and `turn/start`, and add
`thread/set_service_tier` to update or clear the persisted thread default.
Turn overrides take precedence over thread settings and provider config.
Thread read/list expose the persisted tier, and conversation forks inherit it.
Both SDKs expose creation defaults, per-turn overrides, and a thread setter.
