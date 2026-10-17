# roder-ext-runner-blaxel

For an allowlisted outbound address, provision a dedicated gateway in Blaxel and
set `egress_gateway` and the matching concrete `region` in the runner destination.
The provider sends `spec.network.egress = { mode: "dedicated", gateway: ... }`
when creating the sandbox and preserves it in detached session state. Creation
and rejoin refuse a sandbox with a different assignment. Omit `egress_gateway`
to retain shared egress. Gateway assignment cannot be changed in place.

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

Declare additional runtime ports in destination config at creation, for example
`ports = [{ name = "verify-control", target = 4319, protocol = "HTTP" }]`.
Declare at most 16 ports. Protocols are `HTTP` or `TCP`; targets and names must be unique. Blaxel reserves
80, 443 and 8080. Ports are retained in session state for rejoin; requesting ports
does not change an existing sandbox's immutable runtime configuration.

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

## Explicit snapshot and fork client

`forks::BlaxelForkClient` uses an explicit `Redacted` credential and workspace;
it never falls back to the default runner environment. Its snapshot create,
lookup, catalog reconciliation and delete methods use workspace-scoped IDs,
including snapshots whose source has been deleted. Capture public runtime pins
using `qualify_source`, then persist them with the authorized checkpoint.
Snapshot capture and live forks recheck those pins before mutating the provider.
Snapshot forks re-read the snapshot and any surviving source's attachment settings.
Only public shared networking, no attached storage and matching mk3 node-generation
pins are supported. `fork_live` and `fork_snapshot` require a deterministic target,
a generation marker and a fresh maximum lifetime of at most 24 hours.
Credentials never follow redirects.

Before delivering private original-source continuation data, use
`observe_original_source(name, external_id)`. One non-secret GET checks the exact
workspace/name/external ID, ready state, endpoint scope and supported runtime and
attachments. Compare the returned endpoint/configuration to the current runner
session immediately before delivery; the observation is not a scrub, browser
identity or application-admission receipt. `from_workspace` constructs this client
from an explicit source-account profile without default credential lookup.

Persist the allocation intent before dispatch. Mutations are sent once: an
`Outcome::Unknown` requires reconciling the recorded identity, and HTTP 409
requires validating the existing resource rather than adopting it automatically.
A fork receipt proves acceptance only. A delete receipt proves acceptance or
observed absence; confirm accepted deletions through a later lookup.

`inspect_child` distinguishes missing, pending and ready children; it reads admin
environment values only to verify the exact generation marker. `adopt_child`
rechecks ownership, runtime pins, ports, lifetime and endpoint, then creates a
runner session using reads only. It does not wake, redeploy, create directories,
execute processes or refresh standby leases. Inherited `externalId` and application
secrets are discarded. Generic provider rejoin rejects adopted-fork state; after
a restart, re-adopt using the durable allocation intent and checkpoint pins.

`delete_child` and adopted sessions' delete-on-close path recheck the current
generation marker before DELETE, including failed or partially ready children.
Never reuse target names across allocation generations. Accepted deletion still
needs a later lookup to prove absence. Caller-owned intents must survive cancellation.

Provider adoption does not authorize inherited browser controls or supply capacity
scheduling. The caller must rotate/rebind browser controls, fence root cancellation
and perform account/application eligibility checks before executing child work.
The generic runner snapshot capability remains disabled.

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

### Recoverable process identity

`run_durable_command` is an explicit operation for an active original session
created through `for_workspace`. The caller must first retain a unique command ID
and win its own durable launch claim. Blaxel derives a stable provider process
name from 128 bits of SHA-256 of that ID. The live session refuses another durable
start with the same ID even after completion; this is not an idempotent start API
or permission to replay after an unknown result.

A fresh joined session can call `observe_durable_command` with the same ID. It
makes one bounded GET and returns `NotObserved`, `Running`, `Exited` or `Unknown`,
without process logs, environment, a wake request or a keep-alive mutation. A 404
can race delayed creation and does not authorize restart. Provider process state
does not attest to browser readiness, identity, admission or control ownership.
Default-account, paused and adopted-fork sessions refuse these operations.

For worker recovery, use `adopt_original_source(&observation, &options)` after a
scoped original-source observation. It performs another non-secret GET and
requires the same endpoint/runtime/account and a `RUNNING` source. Missing,
changed or standby sources fail without provisioning, waking, lifecycle updates
or process startup. The resulting explicit-workspace session can inspect the
retained durable command; native identity/control and current source authority
must still be checked before private delivery. Adoption uses `Keep` cleanup and
no standby refresh; its `close` does not delete the VM. The durable owner must
separately reconcile authorized physical cleanup.
