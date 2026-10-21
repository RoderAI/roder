---
roder-ext-anthropic: patch
---

# Honor requested Anthropic service tiers and retain the reported tier

Forward explicit runtime service-tier selection to Messages requests, allowing
callers to enforce standard-only service. Preserve the tier reported in usage
without assuming a default when it is absent.
