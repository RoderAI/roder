# Cua Linux integration validation

The native Rust Roder contributor is qualified on Linux x86_64 in Blaxel,
with XFCE/X11, AT-SPI and checksum-pinned Cua Driver 0.34.0. Enable [cua]
in config and bind the thread to a provisioned desktop runner. See
[cua-computer-use.md](cua-computer-use.md) for setup, permissions and app launch.

| Check | Result |
| --- | --- |
| Live model through built Roder public app-server | Passed; exact 6, *, 7, = button sequence with pixel and element clicks plus Escape keypress; actual PNG image blocks in model requests; displayed 42 and independent AT-SPI 42 |
| Native GTK app-owned grader | All 16 input, window, isolation and recovery checks passed |
| Two simultaneous desktop sandboxes | Second app remained untouched; foreign capture and element token rejected |
| Cancellation | Already-started atomic drag completed under its desktop fence; no held button; no late input from abandoned queued worker |
| Cleanup | All three final evaluation sandboxes report TERMINATED |
| Rust Cua | 2 capture/binding tests plus 8 executor/transport boundary tests passed |
| Public ACP | Three Cua wire checks cover approvals, images, runner routing, Plan refusal and failed input retaining its image; full 13-test suite passed serially |
| App-server JSON-RPC e2e | 132 passed, 1 ignored, with live API credentials removed from test process environment |
| Remaining workspace | 3,284 passed, 68 ignored with roder-ext-jev excluded after its fixture failures |
| Provider image/config/registry tests | Passed; Gemini's nullable enum mapping also passed its 19-test lib suite |
| Fixture offline checks | 5 launcher checks and 3 calculation-trace checks passed; earlier feasibility fixture has 6 passing checks |
| Release metadata | Required package changeset present; generated knope config current |
| Formatting | Changed Rust files and git diff --check passed |
| New Cua crate strict Clippy | All targets passed with --no-deps and -D warnings |

Final evidence is committed under
[examples/cua-linux/evidence/2026-10-08](../examples/cua-linux/evidence/2026-10-08/):
actual PNGs, driver observations, tool traces, model image counts, app-owned
graders, sandbox identity and terminal cleanup. The decision model saw only
desktop tools during its calculator task. Neither shell arithmetic nor direct
application-state injection counts as GUI proof. The native primitive fixture
is scripted and records that it does not use a decision model.

The full-workspace run was blocked by four unmodified roder-ext-jev browser
fixtures (450 other Jev checks passed, 9 ignored): keyless_corpus_passes,
typed_secrets_appear_nowhere_a_run_leaves_behind, keyless_session_corpus_passes
and a_reopen_never_loads_a_page_outside_the_operators_origins. Failures included
a CDP Runtime.evaluate send error and incorrect resulting fixture navigation.
The remaining workspace passed separately. An initial process-host cancellation
timing failure passed its isolated retry and the remaining-workspace run.

Global fmt and strict Clippy remain blocked by untouched source: formatting in
hosted/runtime_pool, item_stream, protocol_contract, server and other modules;
Clippy in roder-supergrok-auth/src/lib.rs:294, roder-api catalog constructors
and thread.rs's large enum. Gemini also has existing provider.rs lints at 137
and 276. These files/lines were not changed to repair unrelated work.
An existing native Chrome ACP fixture intermittently read an empty
DevToolsActivePort during concurrent launch; the serial ACP suite passed.

Responses received live model validation. Anthropic and Gemini image replay
are serialization-tested, not live-model qualified here. Gemini's provider
schema converts nullable unions and removes null enum members, matching its
[documented Schema format](https://ai.google.dev/api/generate-content#Schema).

Native Wayland and local macOS remain unqualified. The public desktop base tag
is mutable: durable deployments should publish a provisioned image and pin its
immutable digest. Generic type_text is limited to ASCII and single-window apps
because of the pinned driver; cua_set_value supports exact Unicode replacement
and dialog fields with readback. Refused and uncertain input is never retried.

## Roadmap 113 acceptance audit

The completion audit used the committed native feature, current PR state,
actual fixture traces/PNGs, test logs and independent app-owned graders.
The live calculator harness now verifies the exact four-button sequence;
keypress operands alone cannot satisfy that check. This strengthens the
calculator acceptance without changing the qualified desktop implementation.

| Required outcome | Authoritative evidence |
| --- | --- |
| Launch a native Linux app in the owned Blaxel desktop | fixture.py launches Galculator under the installer's actual graphical/D-Bus session prefix; both native reports identify the owned sandboxes and XFCE/X11 backend |
| Public Roder runtime and live model compute 42 | native-model-final/report.json records extension registration, public app-server, successful 6, *, 7, = clicks, real model image blocks and independent AT-SPI 42; final.png visibly shows 42 |
| Window and full-desktop observations | Native specs expose discovery/window/desktop tools; GTK trace records real desktop capture and cursor input; PNG decoding validates dimensions against metadata |
| Click, double/right click, keys/chords, text, drag and scroll | Exact calculator button trace plus all 16 GTK checks; app-owned event grader verifies the primitives, Unicode replacement, dialog and multiple-window targeting |
| Pixels match scaled screenshots and handles cannot cross targets | GTK scaled drag/scroll proof, two-sandbox foreign capture/token rejection, and executor tests for stale, foreign, wrong-window and out-of-image input |
| Coding and desktop tools share the thread's leased runner | Core workspace tool routing includes cua_*; the runtime holds its runner lock/lease across execution and capture; public ACP tests inspect the selected runner command |
| Plan refusal, Default approvals, existing permissive modes | Native policy contributor and executor Plan guard; public ACP tests prove approved actions, observational Plan access and zero input dispatch in Plan |
| Partial state after failure and no automatic input replay | Refusal boundary and ACP tests keep failed status with a fresh PNG and exactly one action dispatch; failed-capture tests revoke grounding |
| Standby/resume, detach/rejoin, crash and cancellation | App-owned GTK checks record successful pause/resume and rejoin, restart refusal plus recovery, cancelled atomic drag with no held button; abandoned-worker test rejects late dispatch |
| Model-visible untrusted observations and bounded images | Live Responses requests contain image blocks; Anthropic/Gemini serialization and ACP wire checks preserve images; Cua results label desktop content untrusted and enforce PNG/response limits |
| Reviewable artifacts and terminal cleanup | Native model and input evidence retain sandbox IDs, pinned driver version, backend, actual PNGs, observations, app graders and TERMINATED status; forwarding records retain counts/statuses without credentials |
| Canonical opt-in extension and release metadata | roder-ext-cua version 0.1.0, config/registry/CLI wiring, setup/API docs, affected-package changeset, passing changeset gate and generated knope config |
| Named roadmap verification commands | Feasibility harness help, Cua tests, Responses image tests, Blaxel runner suite, public ACP/JSON-RPC suites, fixture checks, roadmap validator and diff checks all passed; the remaining-workspace log includes Responses and Blaxel suites |

The reviewed deliverable is draft PR #119. Merging or releasing is outside this
implementation goal. Optional native Wayland and local macOS qualification
remain explicitly separate. Existing unrelated Jev/global-check failures are
recorded above and do not establish a pass for those checks.
