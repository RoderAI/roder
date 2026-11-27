# Desktop handoff: Roder 0.4.12 and Antigravity subscriptions

## Release identity

| Package | Target release |
| --- | --- |
| `roder` | `0.4.12` |
| `roder-app-server` | `0.7.4` |
| `roder-tui` | `0.4.4` |
| `roder-ext-antigravity-backend` | `0.1.1` |

- Immutable CLI tag: `roder/v0.4.12`.
- Release merge commit: `aaf5773ff0496efc9ee7db5450cc9607b8cab004`.
- Feature PR: RoderAI/roder #234. Version/release PR: #235.
- Versioned release workflow `45915197419` succeeded, including all three native
  builds, macOS signing/notarization, GitHub uploads, Homebrew, and npm publication.
- Public npm `@roderai/roder@0.4.12` and its `latest` dist-tag were verified.
- Mutable `latest` workflow `45915196385` also succeeded; its public manifest
  points to the same release commit, and its Linux x64 CLI is byte-identical to
  the verified versioned release binary.
- The downloaded Linux x64 release archive passed SHA-256, version, schema-export,
  and noninteractive guided-setup safety smoke tests.
- Supporting lockfile versions include `roder-api 0.3.3`,
  `roder-protocol 0.4.2`, `roder-core 0.4.5`, and
  `roder-extension-host 0.7.3`.
- All 22 missing workspace versions were published and independently verified:
  non-yanked versions, archive SHA-256 values, and `.cargo_vcs_info.json`
  provenance matching the release commit.
- A clean standalone dependency on `roder = "=0.4.12"` resolves the expected
  companion versions entirely from the registry, without Git/path fallbacks.
- Publication evidence is recorded locally under
  `.roder/release-artifacts/antigravity/`, especially
  `cargo-publish-results.json`, `registry-verification.json`, and
  `desktop-registry-metadata.json`.

The upstream release was isolated from unrelated dirty OpenClaw, ACP, and theme
work. The original local Roder worktree was not reset or cleaned. Use the
published crate or immutable release tag, not a path dependency on that checkout.

## Scope for the next desktop task

Upgrade the desktop's embedded Roder release and expose the Antigravity agent
backend with its managed installation/Google-account setup flow. Do not cut a
Desktop release until its integration and packaged-app acceptance checks pass.

The inspected desktop checkout is `~/w/roder-desktop`, HEAD `5fd618a`, with
pre-existing dirty files including the distro config, manifest, and lockfile.
Preserve that work. Read its `AGENTS.md`, `docs/design.md`, and
`.agents/skills/roder-distro/SKILL.md` before editing.

## 1. Upgrade the embedded binary, not a local source checkout

Current inspected configuration pins `roder 0.4.11` through the Git tag
`roder/v0.4.11`, despite comments describing a crates.io dependency.

Files:

- `roder-distro-config.toml`
- `roder-desktop-distro/Cargo.toml`
- `roder-desktop-distro/Cargo.lock`
- `scripts/lib/roder-distro.mjs`

**Updater pitfall:** `setDistroVersion()` changes `[roder].version` but derives
the dependency from the existing `[source].tag`. Running
`pnpm roder:distro:update 0.4.12` alone can leave the binary on the old Git tag.

Preferred path after crates.io publication:

1. Remove `[source].tag` so the updater selects the registry dependency.
2. Run the updater and inspect both manifests and the resolved lockfile.
3. Confirm the lockfile resolves `roder 0.4.12` and the package versions above.
4. Alternatively, explicitly set `[source].tag = "roder/v0.4.12"` before running
   the updater when using the immutable-tag route.

```sh
pnpm roder:distro:update 0.4.12
pnpm roder:distro:check
pnpm roder:distro:release
./resources/bin/roder --version
./resources/bin/roder app-server schema --format manifest
```

Commit the source config, distro manifest, and lockfile. Do not commit the
generated `resources/bin/roder` executable. Update the stale embedded-version
paragraph in the desktop README. The distro wrapper's `roder::run_distribution`
entry point remains the supported way to build the embedded CLI.
Upstream release builds were validated with Rust `1.99.1`; use that toolchain
when reproducing upstream builds and investigate any desktop CI toolchain drift.

## 2. Add agent-backend selection and lifecycle handling

`electron/roder/app-server-client.ts` currently starts the bundled sidecar with:

```text
roder app-server --listen stdio://
```

Antigravity requires:

```text
roder app-server --backend antigravity --listen stdio://
```

Backend selection is per app-server process. Passing `modelProvider:
"antigravity"` to a native backend's `thread/start` is not a substitute.

Add an explicit native/Antigravity backend choice, controlled sidecar restart,
and catalog/state refresh. Namespace thread/history state by backend. If
simultaneous native and Antigravity conversations are required, use separate
sidecars and route requests/events by backend ownership.

`providers/list` and `model/list` from the Antigravity sidecar contain the
account's catalog. Replace the prior catalog rather than presenting native API
models that this sidecar cannot execute. Handle missing-runtime and authentication
failures during `initialize` with an actionable setup affordance.
Reasoning controls are model-specific: use advertised values rather than copying
one model's thinking choices across the entire catalog.

Relevant desktop surfaces:

- `electron/roder/app-server-client.ts`
- `src/lib/roder-ipc.ts`
- `src/stores/roder-store.ts`
- `src/components/providers/provider-connect-card.tsx`
- `electron/onboarding/environment.ts`

Keep Antigravity subscriptions distinct from the existing Gemini API-key
provider and Codex account status. Do not import, copy, or parse Google tokens.

## 3. Run setup in an owned PTY

The release provides these commands:

```text
roder setup antigravity
roder auth login antigravity
roder auth status antigravity
roder auth logout antigravity
```

The setup wizard detects an existing runtime or offers an explicit managed
installation. It validates the official release manifest, exact download origin,
SHA-512, archive, and version. It then hands the normal terminal to the official
Google sign-in flow and verifies account access before continuing.

The piped app-server intentionally never installs software or initiates
interactive login. For the first desktop integration:

1. Add a "Set up Antigravity subscription" action.
2. Spawn the **bundled** `roder setup antigravity` in a dedicated, owned PTY.
3. Match `HOME`, `RODER_CONFIG_DIR`, and `RODER_DATA_DIR` with the sidecar.
4. Present the real wizard and official sign-in; do not scrape authorization
   codes or send setup terminal contents to telemetry/session recording.
5. On success, close the setup PTY, restart the chosen backend, and reload models.
6. On cancel/error, retain usable existing installations and clean up the PTY.

`electron/terminal/pty-manager.ts` currently manages a singleton shell terminal.
Add a dedicated command-capable setup session; do not inject a command into a
user's existing shell.

A native graphical installer would need a deliberate bridge to the Rust
`runtime::{find_runtime, managed_directory, install_runtime, runtime_version}`
APIs and progress events. This release does not add graphical-install or login
JSON-RPC endpoints. Do not invent them in the renderer.

Managed executables are stored under the Roder config root's
`tools/antigravity/<platform-target>/` directory and take precedence over PATH.
Account commands and agent sessions use the same official file-backed OAuth
flow, with ambient API-key/cloud-auth selectors removed.

## 4. Preserve permission semantics

- Default, AcceptAll, and Plan startups do not pass native permission bypass.
- Bypass startup launches `agy acp --dangerously-skip-permissions`.
- `--yolo` selects Bypass for a CLI/app-server launch.
- A session launched with native bypass rejects later tightening of its policy.
  Surface the new-session-required error or recreate the backend safely.
- Switching a normally launched session to Bypass uses host-side approval
  handling, rather than changing immutable process arguments.
- Do not silently equate AcceptAll with native Bypass.
- Plan requires the installed agent to advertise a supported plan mode.

Verify that Desktop policy choices reach the actual backend launch/turn paths.
Do not display a safer mode while retaining a native-bypass process underneath.

## 5. Tests and Electron acceptance

Follow the desktop's TDD and UI rules. Add tests for:

- Distro updater Git-tag/registry pin behavior and lockfile versions.
- Backend argv, restart ownership, and catalog replacement.
- Setup PTY success, cancellation, error/retry, and environment parity.
- Subscription/API-key separation and actionable initialization errors.
- Backend-specific thread histories, approvals/questions, interruption/resume.
- Bypass startup and safe refusal of in-place permission tightening.

Existing focused suites:

```sh
pnpm exec vitest run \
  test/roder-app-server-client.test.ts \
  test/roder-ipc-policy-mode.test.ts \
  test/roder-models.test.ts \
  test/roder-model-reasoning.test.ts \
  test/roder-thread-tool-visualization.test.ts \
  test/onboarding.test.ts \
  test/onboarding-store.test.ts \
  test/provider-copy.test.ts

pnpm typecheck
pnpm test
pnpm lint
pnpm format:check
pnpm roder:distro:release
pnpm dev
pnpm make
```

Manual acceptance must run inside Electron, not an ordinary browser pointed at
the renderer URL:

1. Missing runtime -> consent -> install -> Google sign-in -> discovered models.
2. Existing runtime/account skips unnecessary setup.
3. Cancel/retry leaves no orphan setup PTY or agent process.
4. A Gemini file-read task emits real tool activity and streamed assistant text.
5. Approvals/questions, cancellation, resume, and backend switching work.
6. Packaged macOS ARM64 and Windows x64 find/install the correct official runtime.

Upstream proof includes a real fresh managed installation, live Gemini text/tool
execution, cancellation/process cleanup, and isolated tmux sign-in/resize/cancel
regressions. It does not replace the Electron acceptance checks above.

### Current account-verification limitation

An additional post-publication live account probe currently fails authentication
with both the previously built local binary and the downloaded 0.4.12 release
binary. The release archive checksum, version, schema export, and noninteractive
setup safeguards passed. No interactive Google sign-in was attempted during
publication. Recheck/reconnect through the official setup flow before the next
live Electron acceptance run; do not assume registry availability proves account
access. Details are in `published-binary-live-smoke.json` alongside the other
local release evidence.
