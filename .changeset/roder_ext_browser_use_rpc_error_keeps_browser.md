---
roder-ext-browser-use: patch
---

# browser_use keeps the browser when a healthy server answers with a JSON-RPC error

A JSON-RPC error reply from a healthy browser-use server (for example an argument that fails the tool's schema) no longer shuts the server down and drops the owned browser profile, and its error no longer claims the browser was lost. Transport errors, timeouts, a dead server, cancellation and a failed page read after an action still drop the browser and say so. The fake-server integration tests are split into focused modules.
