---
roder-core: patch
---

# Serialize shared remote runner tools and refresh their deadlines

Prevent concurrent agent threads attached to the same remote runner session from interleaving multi-step workspace tool operations. Recompute the tool deadline after lazy runner provisioning and the shared-session queue so command leases use the time that actually remains.
