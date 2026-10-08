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

The model must clear the native calculator, compute 6 * 7, use pixel and
element clicks plus a keypress, and inspect the displayed 42. Roder permits
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
