---
roder-app-server: major
---

Add bounded asynchronous hosted connection policy revalidation at admission, before dispatch and while idle. Denial revokes the connection's executor leases and terminalizes pending calls before closing the socket.

Hosted policy types now live in `hosted::connection_policy` and remain available from `hosted`; callers using the old `hosted::gateway` type paths must update imports.
