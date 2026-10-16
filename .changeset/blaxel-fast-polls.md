---
roder-ext-runner-blaxel: patch
---

# Check Blaxel commands sooner and keep a fresh standby lease

A command is checked 10 ms after it starts, backing off to every 250 ms, instead of waiting 250 ms before the first check; most sandbox commands finish within milliseconds. Operations no longer stop the standby grace lease before they start and start a new one after: a lease that started less than a quarter of its duration ago is kept, saving two round trips per operation. Pause and close still stop it.
