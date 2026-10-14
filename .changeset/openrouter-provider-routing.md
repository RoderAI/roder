---
roder-config: minor
roder-ext-openai-responses: minor
roder-ext-openrouter: minor
roder-extension-host: minor
roder: minor
---

# Configure OpenRouter provider routing and service tiers

OpenRouter requests now route by price by default (`provider.sort = "price"`). `[providers.openrouter.routing]` and per-model `[providers.openrouter.model_routing."<slug>"]` tables replace that default and are sent as OpenRouter's `provider` object, so a model can be pinned to specific upstream providers (`only = ["deepinfra/fp8"]`, `allow_fallbacks = false`), capped with `max_price`, or sorted by throughput or latency; `routing = {}` restores OpenRouter's load balancing. `[providers.openrouter] service_tier` (`flex`, `priority`, `ultrafast`) is sent as the request's service tier, and a per-request runtime service tier now reaches OpenRouter as well.
