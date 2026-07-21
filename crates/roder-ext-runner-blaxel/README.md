# roder-ext-runner-blaxel

`roder-ext-runner-blaxel` is the Blaxel runner provider for [Roder](https://roder.sh).

## What It Does

It runs a Roder thread's coding tools inside a [Blaxel sandbox](https://docs.blaxel.ai/Overview)
— an instant-launching micro VM that scales to standby when idle and resumes in
milliseconds with its filesystem and processes intact. The provider drives the
Blaxel control-plane (`/sandboxes`) and per-sandbox REST APIs (process,
filesystem, preview) and supports the full runner lifecycle: pause, resume,
detach, and rejoin. Long-running commands use named process polling instead of
the synchronous 60-second API window, and timeout or turn interruption
terminates the named supervisor, then uses a separate untagged cleanup process
to TERM/KILL every descendant carrying that command's unique environment tag.
Cancellation succeeds only after no tagged process remains; the finite
server-side lease bounds the named supervisor as a backstop.

The provider can keep the sandbox active for a bounded interval after runner
operations with `standby_after = "5m"`, replacing a single detached keep-alive
lease and cancelling it on pause or close. It also supports validated Blaxel
lifecycle expiration policies such as `{ type = "ttl-idle", value = "7d" }`.
Policies are reconciled in place on create and rejoin, preserving persistent
checkouts and uncommitted work instead of creating a new sandbox generation.

The cleanup proof requires a Linux sandbox image with `/proc`, Python 3 with
`os.pidfd_open` and `signal.pidfd_send_signal`, and permission to read the
environment of extant userspace processes. The scanner opens a pidfd before
validating the exact NUL-delimited tag and signals through that pidfd, avoiding
raw-PID reuse races. It skips only tasks positively identified as Linux kernel
threads, which have no userspace environment. Missing pidfd support or an
inaccessible userspace environment fails closed, so cancellation returns
`false` and remains retryable. The live smoke verifies these requirements for
the default Blaxel base image; custom images must provide them too.

A positive named-process kill or terminal record permits the unique tombstone
to be deleted after the bounded acknowledgement-retention window. If process
registration only times out or is observed absent, the tombstone remains
permanently to fence a late POST; only the local command mapping is reaped after
the complete server-lease horizon. Persistent files under
`/tmp/roder-cancelled-processes` are therefore intentional in that case.

Credentials come from `BLAXEL_API_KEY` (or `BL_API_KEY`) plus `BL_WORKSPACE` and
are never written to session state. See
[`docs/roder-blaxel-runner.md`](https://github.com/RoderAI/roder/blob/master/docs/roder-blaxel-runner.md)
for setup, configuration, and the lifecycle model.

## How It Fits Into Roder

Roder is an agentic software development system with a Rust CLI/TUI, a JSON-RPC app-server, SDKs, package resources, and first-party runtime extensions. This package is released as part of that workspace so downstream users can depend on the same component boundaries that Roder itself uses.

## Links

- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```
