# Roder Fly Sprites Runner

Roder can run remote-runner sessions inside Fly Sprites sandboxes through the first-party `sprites` runner provider.

Sprites are persistent Linux environments. Creating or waking one can create billable remote resources, so use `cleanup = "delete-on-close"` for throwaway runs and `cleanup = "keep"` only when you intentionally want to preserve the environment.

## Environment

Set one of these before starting Roder:

```sh
export RODER_SPRITES_TOKEN=...
export SPRITES_TOKEN="$RODER_SPRITES_TOKEN"
```

`RODER_SPRITES_TOKEN` takes precedence over `SPRITES_TOKEN`. For staging or local fakes, set `RODER_SPRITES_BASE_URL` or `SPRITES_BASE_URL`; otherwise Roder uses `https://api.sprites.dev`.

Live tests are off by default. They only contact Sprites when explicitly enabled:

```sh
RODER_LIVE_SPRITES_RUNNER=1 cargo test -p roder-ext-runner-sprites live_sprites_smoke -- --ignored --nocapture
```

The durable app-server live smoke additionally requires `RODER_REMOTE_APP_SERVER_TOKEN`.
It can either download `remote-roder` from `dl.roder.sh/latest` or upload a local
Linux binary with `RODER_LIVE_SPRITES_REMOTE_RODER_BIN`:

```sh
RUSTC="$(rustup which rustc)" \
  cargo zigbuild --manifest-path dist/remote-roder/Cargo.toml --release --target x86_64-unknown-linux-gnu

RODER_LIVE_SPRITES_RUNNER=1 \
RODER_LIVE_SPRITES_APP_SERVER=1 \
RODER_LIVE_SPRITES_REMOTE_RODER_BIN="$PWD/dist/remote-roder/target/x86_64-unknown-linux-gnu/release/remote-roder" \
RODER_LIVE_SPRITES_REPO_SOURCE="$PWD" \
  cargo test -p roder-ext-runner-sprites live_sprites_repo_app_server_accepts_remote_control -- --ignored --nocapture
```

After publishing `remote-roder-*` to `dl.roder.sh/latest`, rerun the same live
smoke without `RODER_LIVE_SPRITES_REMOTE_RODER_BIN` to prove the Sprite can
download the production artifact itself:

```sh
make publish-verify

RODER_LIVE_SPRITES_RUNNER=1 \
RODER_LIVE_SPRITES_APP_SERVER=1 \
RODER_LIVE_SPRITES_REPO_SOURCE="$PWD" \
  cargo test -p roder-ext-runner-sprites live_sprites_repo_app_server_accepts_remote_control -- --ignored --nocapture
```

By default the live smoke deletes the created sprite. Set `RODER_SPRITES_LIVE_KEEP=1` to keep it for manual inspection.

## Configuration

```toml
[remote_runners]
enabled = true
default_destination = "sprites-dev"

[remote_runners.destinations.sprites-dev]
provider = "sprites"
config = {
  sprite_name_prefix = "roder",
  url_auth = "sprite",
  cleanup = "delete-on-close",
  working_dir = "/home/sprite/roder",
  network_policy = { default = "deny", allow = ["github.com", "*.npmjs.org", "crates.io", "static.crates.io", "dl.roder.sh"] }
}
```

Use `sprite_name = "existing-sprite"` to reuse a known sprite. User-named sprites default to `cleanup = "keep"`; generated sprites default to `delete-on-close`.

Do not put raw tokens in config. The CLI rejects secret-looking config keys; use environment variables.

## Forking A Repo Into A Sprite

Roder runner manifests can materialize a local source directory into the Sprite working directory before commands or the remote app-server service starts. Use a directory manifest entry when the intended remote workspace is the current repo:

```toml
# Conceptual destination manifest shape used by Roder runtime callers.
[[remote_runners.destinations.sprites-dev.default_manifest.entries]]
source = "."
target = "repo"
writable = true
```

The Sprites provider archives directory manifests into a tar file, uploads that archive once, and extracts it under `target` inside the Sprite. This avoids thousands of filesystem API calls for repo-sized forks while preserving workspace-relative paths. It skips generated or high-churn directories at the source root: `.git`, `.hg`, `.svn`, `.roder`, `.codex`, `target`, `node_modules`, `.next`, `dist`, `build`, and `roadmap/assets`. This keeps generated Sprites small enough for agent work while avoiding VCS internals, local build artifacts, and generated media. Package manager caches and dependency installs should happen inside the Sprite after the fork, subject to `network_policy`.

## Remote App-Server Nodes

Sprites can host a durable headless Roder node. Enable `app_server` to have the provider install the remote-only Roder distribution and run `roder app-server --remote` as a Sprites managed service:

```toml
[remote_runners.destinations.sprites-dev]
provider = "sprites"
config = {
  sprite_name_prefix = "roder",
  cleanup = "keep",
  working_dir = "/home/sprite/roder",
  network_policy = { default = "deny", allow = ["dl.roder.sh", "api.openai.com"] },
  app_server = {
    enabled = true,
    download_base_url = "https://dl.roder.sh/latest",
    binary_name = "remote-roder",
    service_name = "roder-app-server",
    port = 17373,
    workspace_path = "repo",
    auth_token_env = "RODER_REMOTE_APP_SERVER_TOKEN",
    env_passthrough = ["OPENAI_API_KEY", "RODER_SPRITES_TOKEN"]
  }
}
```

Build the binary with the `remote-app-server` distribution profile and publish it as `remote-roder-<target>` plus `remote-roder-<target>.sha256`, for example `remote-roder-x86_64-unknown-linux-gnu`. The `publish-latest-roder.yml` workflow publishes these Linux remote distro artifacts to `dl.roder.sh/latest`; locally, `make publish` does the same when `CLOUDFLARE_API_TOKEN` is set. The local publish script sets `RUSTC="$(rustup which rustc)"` for `cargo zigbuild` when rustup is available, so Homebrew `rustc` does not break Linux builds. The Sprite bootstrap resolves its Linux target with `uname`, downloads and verifies the artifact, installs it as `.roder/bin/roder`, and creates or reuses the running `roder-app-server` service.

Use `make publish-verify` after publishing to confirm the public manifest and required Linux `remote-roder-*` binary/checksum URLs exist before running the production-download live smoke. CI runs the same verifier after upload with `RODER_PUBLISH_VERIFY_ATTEMPTS` and `RODER_PUBLISH_VERIFY_DELAY_SECONDS` set so public endpoint propagation has time to settle.

Set `app_server.workspace_path` to the manifest target that contains the forked repo, for example `repo`. The provider starts the app-server service in `/home/sprite/roder/repo` while invoking the installed binary from `/home/sprite/roder/.roder/bin/roder`, so Roder tools operate against the forked workspace. If omitted, the app-server runs in the destination `working_dir`.

For local development only, set `app_server.local_binary_path` to upload a local binary instead of downloading from `dl.roder.sh`.

The auth token value is read from `RODER_REMOTE_APP_SERVER_TOKEN` locally and injected into the service environment. Session state records the service name, port, Sprite URL, remote WebSocket `connect_url`, `/readyz` health URL, proxy endpoint, token env name, supported auth schemes, subprotocol templates, and status, but not the token value. Native clients connect with `Authorization: Bearer $RODER_REMOTE_APP_SERVER_TOKEN`; browser-constrained clients use `Sec-WebSocket-Protocol: roder.remote.v1, bearer.$RODER_REMOTE_APP_SERVER_TOKEN`.

## Supported Operations

- Session lifecycle: create, reuse, resume from non-secret state, and close/delete according to cleanup mode.
- Fresh sessions create the configured working directory before the first command runs.
- Remote app-server bootstrap: optional `app_server` config installs the `remote-roder` distribution and starts a Sprites managed service for durable headless operation.
- Commands: maps Roder `RunnerCommandRequest` to Sprites HTTP POST exec with repeated `cmd` query parameters, cwd, and environment values. The live POST exec response is decoded as non-TTY binary stream frames.
- Files: workspace-relative read/write through Sprites filesystem endpoints with parent directory creation on writes. Transient live API `404`, `502`, `503`, and `504` write responses are retried with bounded backoff during repo materialization and binary upload.
- Manifest materialization: file entries and source directory entries are uploaded before the session is returned, so repo-like workspaces can be forked into the Sprite.
- Checkpoints: maps Roder snapshots to Sprites checkpoint creation and stores only non-secret snapshot metadata.
- Ports: returns the sprite URL when available. Managed app-server sessions expose a direct WebSocket `connect_url` through the Sprites service `http_port`; generic WebSocket TCP proxying for arbitrary runner ports is intentionally deferred until Roder has a local preview relay contract.
- Artifacts: file export is represented as provider metadata; recursive directory export is not implemented yet.

Roder remains the policy authority for approvals and path scope. The Sprites network, privilege, resource, connector, and URL-auth settings are defense-in-depth provider configuration.

## Troubleshooting

- `sprites runner requires RODER_SPRITES_TOKEN...`: export the token before launching Roder.
- `runner path cannot escape workspace`: use workspace-relative paths; absolute paths and `..` are rejected before requests leave Roder.
- `delete sprite` failures on close: check the Sprites dashboard for the sprite name in the runner session state and delete it manually if needed.
- Package installs fail inside the sandbox: expand `network_policy.allow` to include the package registry domains you need.
- `sprites app-server bootstrap failed`: confirm `dl.roder.sh` is reachable from the Sprite, the `remote-roder-<target>` artifact and `.sha256` exist, and `RODER_REMOTE_APP_SERVER_TOKEN` is exported before starting local Roder.
