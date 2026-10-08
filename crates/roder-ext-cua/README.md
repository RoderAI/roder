# Roder Cua desktop extension

A native Rust extension for computer use on a Linux desktop inside the thread's
remote runner. The qualified launcher uses Cua Driver **0.34.0**, XFCE/X11 and
Blaxel. The model uses ordinary cua_* function tools and receives real PNGs.
Roder owns inference, approvals, thread isolation and the sandbox lifecycle.

See [setup and contract](../../docs/cua-computer-use.md) and
[the calculator fixture](../../examples/cua-linux/README.md).

The extension is opt in. It has no local desktop fallback, sandbox-selection
tool argument, desktop HTTP port, or separate Blaxel credential store. Calls
use the execution context's remote workspace, fence and cancellation.

Only atomic input operations are exposed. Stale/foreign capture handles,
unknown arguments, mismatched contexts and ungrounded input fail before native
dispatch. Plan mode observes and denies input; default mode requests approval;
Accept All and Bypass retain Roder's standard approval semantics. Every input
is followed by a fresh window/desktop capture. Uncertain input is never retried.
