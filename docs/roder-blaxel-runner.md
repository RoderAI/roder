# Blaxel sandbox runner

The `blaxel` remote-runner provider runs a Roder thread's coding tools inside a
[Blaxel sandbox](https://docs.blaxel.ai/Overview): an instant-launching micro VM
that scales to zero (standby) when idle and resumes in under ~25 ms with its
entire filesystem and running processes preserved. The provider supports the
full runner lifecycle — **pause**, **resume**, **detach**, and **rejoin** — so a
sandbox can outlive a single turn or even a Roder process restart.

## Setup

Create an API key in the [Blaxel console](https://app.blaxel.ai/profile/security)
and note your workspace name (visible in the console URL `app.blaxel.ai/{workspace}`).

| Variable | Purpose |
| --- | --- |
| `BLAXEL_API_KEY` (or `BL_API_KEY`, or `RODER_BLAXEL_API_KEY`) | Bearer API key |
| `BL_WORKSPACE` (or `RODER_BLAXEL_WORKSPACE`) | Blaxel workspace name |
| `BLAXEL_RUNNER_BASE_URL` / `RODER_BLAXEL_BASE_URL` | Override control-plane base URL (default `https://api.blaxel.ai/v0`) |

The credential is always read from the environment and is **never** written to
thread metadata, session state, logs, or error messages.

## Configuration

```toml
[remote_runners]
enabled = true
default_destination = "blaxel-dev"

[remote_runners.destinations.blaxel-dev]
provider = "blaxel"
secret_env = { BLAXEL_API_KEY = "BLAXEL_API_KEY", BL_WORKSPACE = "BL_WORKSPACE" }
config = { image = "blaxel/base-image:latest", memory = 4096, region = "us-pdx-1", working_dir = "/home/user/roder", cleanup = "detach-on-close" }
```

Config keys (all optional except where the sandbox cannot start without them):

| Key | Default | Notes |
| --- | --- | --- |
| `sandbox_name` | generated from the thread id | Reuse an existing sandbox |
| `sandbox_name_prefix` | `roder` | Prefix for generated names |
| `external_id` | thread destination id | Caller-owned id for rejoin recovery |
| `image` | `blaxel/base-image:latest` | Sandbox image / template |
| `memory` | `4096` | Memory in MB (also sets CPU) |
| `region` | nearest | e.g. `us-pdx-1`, `eu-lon-1` |
| `ttl` | none | Max-age before auto-deletion, e.g. `24h` |
| `working_dir` | `/home/user/roder` | Sandbox working directory |
| `cleanup` | see below | `delete-on-close`, `detach-on-close`, or `keep` |

`cleanup` defaults to `delete-on-close` for freshly created sandboxes and
`detach-on-close` when reusing an existing `sandbox_name`.

### Routing tools into the sandbox

Setting Blaxel as the active runner — via `default_destination` in config or the
TUI runner picker (`Ctrl+P` → Runners → blaxel) — auto-binds new threads so their
coding tools (shell, file read/write, apply-patch) execute **inside** the
sandbox at `working_dir` (default `/home/user/roder`), not on the local machine.
Bindings apply to threads created after selection; the simplest way to bind the
first thread is to set `default_destination = "<your-blaxel-destination>"` in
config so it is active at startup.

## Lifecycle model

Blaxel has no explicit stop/start API: a sandbox stays active while a connection
is held and transitions to standby ~15 s after the last connection drops,
resuming on the next request. Roder maps the runner lifecycle onto this model:

- **pause** — mark standby intent so the sandbox scales to zero; the next
  command transparently wakes it.
- **resume** — wake a standby sandbox immediately (a lightweight health ping).
- **detach** — persist the durable sandbox identity to the thread and release
  the local session, leaving the sandbox alive and rejoinable.
- **rejoin** — reattach to the same sandbox from persisted thread state. Roder
  prefers the persisted sandbox name and falls back to Blaxel's
  get-by-external-id lookup, so a non-terminated sandbox is recovered without
  provisioning a new one.
- **close** — honors `cleanup`: delete the sandbox (`delete-on-close`) or leave
  it on standby (`detach-on-close` / `keep`).

Memory and filesystem state survive standby automatically; there is no separate
snapshot artifact, so `snapshots` is reported as unsupported. For durability
beyond standby memory, attach a Blaxel [volume](https://docs.blaxel.ai/Volumes/Overview).

## Driving the lifecycle

App-server JSON-RPC methods (thread-scoped; params use `thread_id`):

- `runners/pause`, `runners/resume`, `runners/detach`, `runners/rejoin`
  (the last accepts an optional `sandbox` to override the persisted name).

CLI:

```sh
roder runners list
roder runners pause  <thread-id>
roder runners resume <thread-id>
roder runners detach <thread-id>
roder runners rejoin <thread-id> [--sandbox <name>]
```

`runners/list` surfaces each provider's capabilities, including `pause/resume`
and `detach/rejoin`, so clients can hide unsupported actions.

## Ports and previews

`expose_port` creates a Blaxel preview URL (`*.preview.bl.run`) for a sandbox
port. Bind your server to `0.0.0.0`. Previews have a 15-minute connection
timeout.

## Live smoke

A gated end-to-end smoke (`RODER_LIVE_BLAXEL_RUNNER=1`, with `BLAXEL_API_KEY`
and `BL_WORKSPACE` set) exercises create → exec → pause → resume → detach →
rejoin → delete:

```sh
RODER_LIVE_BLAXEL_RUNNER=1 cargo test -p roder-ext-runner-blaxel --test live -- --ignored
```

Offline contract tests run by default and fail if the live API is contacted:

```sh
cargo test -p roder-ext-runner-blaxel
```
