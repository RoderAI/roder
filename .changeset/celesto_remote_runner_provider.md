---
roder-ext-runner-celesto: minor
roder-extension-host: minor
---

# Add Celesto sandboxed-computer remote-runner provider

Add `roder-ext-runner-celesto`, a first-party remote-runner provider that runs a
thread's coding tools inside a Celesto sandboxed "computer" over the Celesto
Computers REST API: create/reuse a computer, exec shell commands, read and write
files, publish preview ports, and manage lifecycle (delete/stop/keep on close).
Configure it with `CELESTO_API_KEY` (or `RODER_CELESTO_API_KEY`) and an optional
`CELESTO_BASE_URL`, and select the `celesto` provider in `[remote_runners]`. The
provider is registered by default in `roder-extension-host`.
