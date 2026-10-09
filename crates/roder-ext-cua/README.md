# Roder Cua desktop extension

A native Rust extension for computer use on a Linux desktop inside the thread's
remote runner or an explicitly selected local macOS desktop. The qualified launcher uses Cua Driver **0.34.0**, XFCE/X11 and
Blaxel. The model uses ordinary cua_* function tools and receives real PNGs.
Roder owns inference, approvals, thread isolation and the sandbox lifecycle.

See [setup and contract](../../docs/cua-computer-use.md) and
[the Linux fixture](../../examples/cua-linux/README.md) and
[the macOS fixture](../../examples/cua-macos/README.md).

The extension is opt in; backend `runner` is the default. `local-macos` selects
the signed app-owned daemon explicitly and rejects remote runner contexts.
Calls use the runner workspace/fence or the shared local desktop fence. The
model cannot select backends, sockets, programs or sessions.

Only atomic input operations are exposed. Stale/foreign capture handles,
unknown arguments, mismatched contexts and ungrounded input fail before native
dispatch. Plan mode observes and denies input; default mode requests approval;
Accept All and Bypass retain Roder's standard approval semantics. Every input
is followed by a fresh window/desktop capture. Uncertain input is never retried.

Browser controls use the same desktop session and policy: exact-window bind,
semantic page snapshots/PNGs, profile preparation, navigation, click, Unicode
typing and explicit session cleanup. Existing-profile attachment requires
`allow_existing_browser_profile=true` plus an independent Cua daemon launch
grant. Native input invalidates browser refs. The full X11 browser fixture uses
Chrome and a sandbox-local test login; personal profiles are never imported.
