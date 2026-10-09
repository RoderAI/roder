# Native macOS Cua qualification

This uses Roder's native `roder-ext-cua` contributor and Cua Driver 0.34.0's
signed app-owned daemon. The tested host is Apple Silicon, macOS 26.6.2.
The Linux/Blaxel backend remains separately qualified; native Wayland still
needs its own compositor qualification.

Build Roder with `mise exec -- cargo build -p roder --bin roder`. Install the
checksum-pinned signed release with Python 3.12 or newer:

```sh
python3 examples/cua-macos/install.py
/Applications/CuaDriver.app/Contents/MacOS/cua-driver permissions grant
open -g /Applications/CuaDriver.app --args serve
/Applications/CuaDriver.app/Contents/MacOS/cua-driver permissions status --json
```

Enable Accessibility and Screen & System Audio Recording for **CuaDriver** in
System Settings when macOS asks. On Tahoe, grant and verify direct capture too.
Cua captures screen video without system audio. The installer preserves an
existing matching app and rejects replacing another release. It neither
modifies TCC databases nor installs a global CLI symlink. Launch through the
signed app with `open`; a terminal-owned raw `serve` process has a different
privacy identity and is not this qualified setup.

Configure Roder explicitly:

```toml
[cua]
enabled = true
backend = "local-macos"
timeout_ms = 45000
max_image_dimension = 1280
```

The default client is `/Applications/CuaDriver.app/Contents/MacOS/cua-driver`;
the default daemon socket is `$HOME/Library/Caches/cua-driver/cua-driver.sock`.
`program` and `socket_path` can select an independently installed signed app
and its app-owned daemon. Do not attach a remote runner to a local macOS thread.
Roder rejects that combination before touching the host desktop.

For the live model acceptance, export `OPENAI_API_KEY` and open Calculator.
Use `cua_list_windows` to select its exact PID/window ID, then run:

```sh
examples/cua-macos/script/build_and_run.sh /tmp/roder-cua-mac-fixture --build-only
python3 examples/cua-macos/live_smoke.py \
  --roder /absolute/path/to/built/roder \
  --pid CALCULATOR_PID --window-id CALCULATOR_WINDOW_ID \
  --grader /tmp/roder-cua-mac-fixture/calculator_grader \
  --output /tmp/roder-cua-mac-model-evidence
```

The model uses only the provided Calculator window and desktop tools. It clears
with Escape, clicks `6`, multiply, `7`, equals in order with both pixel and
semantic clicks, and sees actual image blocks. A read-only Apple Vision grader
checks `42` in the final PNG's display region. The harness stores only Calculator
window pixels and window-subtree evidence, counts/statuses of model requests,
and closes its own app-server. It leaves the chosen Calculator open.

The scripted AppKit fixture independently records events and editable values:

```sh
examples/cua-macos/script/build_and_run.sh /tmp/roder-cua-mac-fixture
mise exec -- cargo build -p roder-ext-cua --example macos_input_smoke
RODER_CUA_MACOS_STATE=/tmp/roder-cua-mac-fixture/state.json \
RODER_CUA_MACOS_OUTPUT=/tmp/roder-cua-mac-input-evidence \
  /absolute/path/to/built/examples/macos_input_smoke
```

It covers double/right click, scaled drag and scroll, exact Unicode typing,
sibling-window replacement, keyboard chords, activation/resize (within the driver's two-point readback tolerance), dialog
replacement/submission, full-desktop capture and scaled cursor placement,
cancelled atomic drag, shared-desktop screenshot invalidation and fresh runtime
grounding. Full-desktop pixels and unrelated window inventories are exercised
without being persisted in evidence. Optionally set `RODER_CUA_MACOS_OWNED_DAEMON_PID` to the exact PID of a daemon
you started for this evaluation to test PID-bound stop/restart, stale-handle
refusal and capture recovery. Leave it unset when sharing an existing daemon.
The test never broadly kills Cua processes. Stop only the fixture PID in `state.json`
after confirming it belongs to this output directory's app bundle.

macOS 0.34 drag and modifier input require explicit foreground permission.
For pointer fixtures bring the owned target to the front first, so AppKit can
consume the events without a temporary activation being restored too soon.
Use a pixel click to focus text fields; they may not implement AXPress.
`cua_set_value` performs a background semantic replacement and has no
`delivery_mode` argument. Generic macOS Unicode typing is qualified with
foreground delivery; background keyboard input may refuse same-PID ambiguity.
These limitations are surfaced; Roder does not silently change delivery or
retry uncertain input.

A cancelled caller does not cancel an already-dispatched atomic gesture. Its
worker holds the process-wide desktop fence until completion. If the reply is
lost or its worker deadline expires, subsequent calls fail closed: restart the
app-owned daemon **and Roder**, then observe again. Multiple independent Roder
processes share the physical desktop but do not share Roder's in-process fence;
use one local Roder process for desktop automation.
