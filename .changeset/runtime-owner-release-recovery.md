---
roder-core: patch
roder-app-server: patch
---

# Recover hosted owner drain after unconfirmed lease release

Retain sealed owner confirmation state across errors, timeouts and cancellation.
Idle expired owners can prove local quiescence without restoring authority;
active execution and failed lifecycle persistence still block completion.
