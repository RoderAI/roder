# Cua Linux integration validation

The native Rust Roder contributor is qualified on Linux x86_64 in Blaxel,
with XFCE/X11, AT-SPI and checksum-pinned Cua Driver 0.34.0. Enable [cua]
in config and bind the thread to a provisioned desktop runner. See
[cua-computer-use.md](cua-computer-use.md) for setup, permissions and app launch.

| Check | Result |
| --- | --- |
| Live model through built Roder public app-server | Passed; successful pixel and element clicks plus keypress; actual PNG image blocks in model requests; displayed 42 and independent AT-SPI 42 |
| Native GTK app-owned grader | All 16 input, window, isolation and recovery checks passed |
| Two simultaneous desktop sandboxes | Second app remained untouched; foreign capture and element token rejected |
| Cancellation | Already-started atomic drag completed under its desktop fence; no held button; no late input from abandoned queued worker |
| Cleanup | All three final evaluation sandboxes report TERMINATED |
| Rust Cua | 2 capture/binding tests plus 8 executor/transport boundary tests passed |
| Public ACP | Three Cua wire checks cover approvals, images, runner routing, Plan refusal and failed input retaining its image; full 13-test suite passed serially |
| App-server JSON-RPC e2e | 132 passed, 1 ignored, with live API credentials removed from test process environment |
| Remaining workspace | 3,284 passed, 68 ignored with roder-ext-jev excluded after its fixture failures |
| Provider image/config/registry tests | Passed; Gemini's nullable enum mapping also passed its 19-test lib suite |
| Launcher offline checks | 5 passed; earlier feasibility fixture has 6 passing checks |
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
