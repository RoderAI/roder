# Native Roder / Cua Linux calculator

This fixture uses the built Rust Roder binary, its public app-server, the
native roder-ext-cua contributor and a live decision model. It independently
reads the calculator's application-owned AT-SPI display after GUI input. A
local forwarding proxy records only model IDs, HTTP statuses and image counts.

Create an **owned disposable** Blaxel sandbox with image
blaxel/cua-xfce:latest, 4096 MiB and a bounded TTL. Record its name. This
upstream template is mutable; the installer separately verifies the exact
Cua 0.34.0 release archive against its published SHA-256.

From the repository root, using an authenticated Blaxel CLI:

~~~sh
python3 examples/cua-linux/provision.py --sandbox YOUR_SANDBOX --workspace YOUR_WORKSPACE
~~~

The installer needs root inside the sandbox. It attaches the daemon and
calculator to the actual cua user's XFCE, DISPLAY and accessibility D-Bus.
It installs /opt/roder-cua/bin/cua-call and exposes no additional port.

Build with "mise exec -- cargo build -p roder --bin roder". Export OPENAI_API_KEY
and BLAXEL_API_KEY (or BL_API_KEY) without putting credentials in files or
arguments. Supply the absolute built binary path:

~~~sh
python3 examples/cua-linux/live_smoke.py \
  --roder /absolute/path/to/roder \
  --sandbox YOUR_SANDBOX --workspace YOUR_WORKSPACE \
  --output /absolute/path/to/evidence
~~~

**This smoke deletes the supplied sandbox**, including after a failed turn.
Use only a sandbox created for this evaluation. It checks asynchronous deletion
until the metadata reports TERMINATED and records that status separately.

The model must clear the native calculator with an Escape keypress, click
6, *, 7 and = in order using both pixel and element routes, and inspect the
displayed 42. The harness independently checks that exact button sequence
against fresh window observations; keypress operands do not satisfy it. Roder permits
foreground input in this owned evaluation after an explicit background refusal.
The harness saves the actual PNGs, tool trace, model image-count evidence,
independent AT-SPI result and cleanup status. No shell tool is advertised to
the decision model.

The earlier process-extension feasibility spike and its proof remain under
examples/non-rust-extensions/cua-linux; they are distinct from this native
runtime acceptance.

Native Wayland and local macOS are not qualified by an X11 run. Their launcher
must establish a suitable graphical session; see Cua's compositor-specific
platform support before claiming support.

The native GTK fixture additionally qualifies double/right click, scaled pixel
drag and scroll, Unicode replacement, key chords, focus, cursor movement,
window resizing and dialog targeting. It records events inside the app, then
checks pause/resume, detach/rejoin, stale capture rejection after a driver
restart, and cancellation with no held button. A second desktop must remain
untouched.

Provision two owned desktops with the same setup command, adding
--input-fixture for each (this installs and launches input_fixture.py).
Build the Rust example with "mise exec -- cargo build -p roder-ext-cua
--example input_smoke" and run:

~~~sh
python3 examples/cua-linux/input_smoke.py \
  --binary /absolute/path/to/debug/examples/input_smoke \
  --sandbox FIRST_OWNED_SANDBOX --sandbox SECOND_OWNED_SANDBOX \
  --workspace YOUR_WORKSPACE --output /absolute/path/to/evidence
~~~

**Both supplied sandboxes are deleted**, including after a failure. This
scripted native primitive fixture complements the separate live model calculator
run; its report explicitly records that it does not use a decision model.

Offline launcher boundaries: "python3 -m unittest discover -s
examples/cua-linux -p 'test_*.py'". Rust boundaries and public ACP behavior:
"mise exec -- cargo test -p roder-ext-cua" and
"mise exec -- cargo test -p roder-app-server --features e2e-tests --test acp".

Shared public app-server/model and calculator trace helpers are in
[examples/cua-desktop](../cua-desktop/); their offline checks use
`python3 -m unittest discover -s examples/cua-desktop -v`.
The native `cua_set_value` tool performs a semantic replacement without a
`delivery_mode` argument. Linux Unicode/sibling-window generic typing limits
remain; macOS native typing is separately qualified.

## Watch the desktop live

Run the local viewer against an explicitly owned, provisioned desktop:

~~~sh
python3 examples/cua-linux/desktop_preview.py \
  --sandbox YOUR_SANDBOX --workspace YOUR_WORKSPACE
~~~

Open the printed `http://127.0.0.1:PORT/` URL in Codex or your browser. The
viewer shows the entire X11 screen, normally refreshing every 2–5 seconds.
It includes a pause button and the age of the latest real frame. If capture
fails or the sandbox stops, the last frame remains visible and the status
shows the loss of live connectivity.

The server binds only to loopback. Blaxel authentication stays in the local
CLI; neither credentials nor a remote desktop-control endpoint enter the
page. Watching uses read-only GDK screen capture, independently of Cua, so
it never changes the agent's capture IDs or sends desktop input. This is
a viewing aid; acceptance evidence continues to use actual Cua observations.
Stop the viewer with Ctrl+C. It does not delete the sandbox.
Live capture stops after one hour by default (`--duration` sets seconds).
The page then serves its last frame with a "Preview ended" status and sends
no further cloud requests. Restart the viewer to begin another viewing period.

## Full XFCE desktop workflow

Provision a fresh owned sandbox with Thunar and LibreOffice Writer:

~~~sh
python3 examples/cua-linux/desktop_fixture.py \
  --sandbox YOUR_SANDBOX --workspace YOUR_WORKSPACE
python3 examples/cua-linux/desktop_smoke.py \
  --roder /absolute/path/to/roder \
  --sandbox YOUR_SANDBOX --workspace YOUR_WORKSPACE \
  --output /absolute/path/to/evidence
~~~

The profile uses 1920×1080 so Writer's Save dialog fits. Provisioning creates
only a source note and empty Archive/Documents directories. The live model
uses the public Roder app-server with only `cua_*` tools to launch apps through
Application Finder, navigate Thunar, drag the note into Archive, create an ODT
report, save it, close Writer and reopen the document through File Manager.
A separate read-only grader checks that the source disappeared, the moved
file's bytes were preserved, and the real ODT contains the unique run marker.
The trace must show the saved document closing, a subsequent File Manager
open action in Documents, and a fresh Writer observation of that report.
LibreOffice may reuse the same native window when returning from Start Center.

Retained reports can be audited with the current ordered trace checks without
replaying any input:

~~~sh
python3 examples/cua-linux/audit_desktop.py /absolute/path/to/report.json
~~~

The audit preserves the original acceptance result and identifies its checks
separately; it cannot repair a failed GUI action or a failed independent grader.

The harness checkpoints its trace and PNGs while running. It deletes the
supplied owned sandbox on completion or failure by default. `--retain-sandbox`
keeps it for explicit additional evaluation or live viewing; its caller must
clean it up. Never rerun the acceptance workflow on an already moved file or
existing report. A failed UI operation is recorded separately from a passing
independent grader; a screenshot alone cannot pass this workflow.

For native input/recovery qualification on this full profile, provision two
fresh desktops with `desktop_fixture.py --input-fixture`, then use the same
`input_smoke.py` command above. These checks operate the app-owned GTK oracle
through Roder and delete both supplied sandboxes. The live document workflow
and primitive/recovery workflow have separate reports and graders.

## Browser on the full X11 desktop

Provision a fresh full desktop with `desktop_fixture.py`, then add Chrome and a
loopback-only test website. Browser setup installs Google's official stable
Debian package, records its version/hash, forces the native accessibility
bridge and renderer accessibility, and verifies native registration. The
browser uses a disposable sandbox-local default profile. No real account or
user profile is imported. The Chrome package is discovered at provision time;
pin the qualified version/hash in a durable image.

~~~sh
python3 examples/cua-linux/desktop_fixture.py --sandbox OWNED --workspace WORKSPACE
python3 examples/cua-linux/browser_fixture.py --sandbox OWNED --workspace WORKSPACE
python3 examples/cua-linux/browser_smoke.py \
  --roder /absolute/path/to/built/roder --sandbox OWNED --workspace WORKSPACE \
  --output /tmp/roder-browser-evidence
~~~

The live test enables `allow_existing_browser_profile` in isolated Roder config
and installs the independent existing-profile grant in this owned fixture's
daemon deployment. It signs in with a native GUI click, attaches the same
running profile, navigates and fills a Unicode form using Cua browser controls,
then uses the native address bar to revisit the receipt. A separate read-only
website event/form-state grader verifies authentication and the exact submitted
values. All page state changes must come through advertised Cua tools.

The smoke rejects a previously signed-in/submitted fixture, records screenshots
and public-runtime calls, ends the Cua browser session, and deletes only the
explicit supplied owned sandbox by default. `--retain-sandbox` keeps it for live
viewing/further evaluation with caller-owned cleanup and a bounded TTL. Use
`desktop_preview.py` to watch the same desktop while the model runs.

Chrome runs with `--no-sandbox` inside this explicitly disposable Blaxel test
container because its process sandbox is unavailable there. This is a fixture
constraint; the production Roder contributor neither launches Chrome itself nor
adds that flag. Only the local fixture website is used for acceptance.
