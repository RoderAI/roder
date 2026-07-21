---
roder-api: patch
roder-core: patch
---

# Add provider-authoritative remote workspace execution leases

Allow remote runner providers to fence a complete multi-step workspace tool
execution across runtimes or replicas. Roder now bounds lease acquisition,
stops execution if the provider loses the fence, releases it on every normal
tool outcome, and refreshes command deadlines after waiting for the fence.
