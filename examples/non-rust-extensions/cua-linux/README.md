# Cua Linux calculator exploration

This process-extension prototype runs a pinned Cua Driver inside one disposable
Blaxel XFCE sandbox. It contributes native window/desktop capture, click and
keypress tools, and translates PNG captures to Roder's `data.__view_image`
payload. It uses the authenticated `bl` CLI and Python's standard library.

The 2026-10-08 live smoke completed `6 × 7 = 42` in **Galculator on Blaxel
Linux/X11**. It used semantic clicks, capture-bound coordinate clicks and a
foreground keypress. The final capture showed 42 and a separate AT-SPI reader
confirmed it. The installed Roder app-server loaded and initialized the
extension through `extensions/list`.

Saved proof: [`evidence/2026-10-08/report.json`](evidence/2026-10-08/report.json),
[`final.png`](evidence/2026-10-08/final.png),
[`desktop.png`](evidence/2026-10-08/desktop.png),
[`host-registration.json`](evidence/2026-10-08/host-registration.json), and
[`cleanup.json`](evidence/2026-10-08/cleanup.json). The evaluation sandbox was
deleted through the Blaxel API; a subsequent authenticated GET confirmed its
terminal `TERMINATED` state. The API still retained its metadata at that check.

The calculator smoke drives the child over JSON-RPC, without a model. The
host smoke verifies registration, without a runtime turn. Native Wayland,
model-visible provider replay, full primitive coverage, cancellation and
multi-thread runner ownership remain acceptance work in
[`roadmap/113-roder-cua-linux-computer-use.md`](../../../roadmap/113-roder-cua-linux-computer-use.md).

## Reproduce

Authenticate `bl` in the intended workspace. Create a new sandbox from
`blaxel/cua-xfce:latest`, with 4096 MB, a one-hour TTL, and the standard sandbox
API port. This image is for discovery: a durable image should pin its digest.
Do not use an existing customer or verification sandbox.

Run these commands from the Roder checkout, replacing `OWNED-SANDBOX` and
`WORKSPACE` with the returned names:

```sh
python3 examples/non-rust-extensions/cua-linux/bootstrap.py \
  --sandbox OWNED-SANDBOX --workspace WORKSPACE
python3 examples/non-rust-extensions/cua-linux/calculator_smoke.py \
  --sandbox OWNED-SANDBOX --workspace WORKSPACE \
  --output evals/reports/cua-calculator/new-run
python3 examples/non-rust-extensions/cua-linux/host_smoke.py \
  --sandbox OWNED-SANDBOX --workspace WORKSPACE \
  --output evals/reports/cua-calculator/new-run/host-registration.json
bl delete sandbox OWNED-SANDBOX --workspace WORKSPACE
```

The bootstrap verifies the SHA256 of the exact `cua-driver-rs-v0.34.0` Linux
x86_64 binary. It installs Galculator and the independent grader's AT-SPI
bindings, then launches Cua and the app as the desktop's `cua` user with its
display authorization and D-Bus session. It refuses to replace an existing
daemon. Do not publish ports 8000, 5901 or 6901 for this test: control stays
inside the authenticated sandbox process API.

The smoke records actual observations and images. It re-observes before each
action and never computes or injects the answer with a shell command. Its
keyboard fallback happens only after Cua explicitly returns
`background_unavailable`; uncertain delivery is never retried. TigerVNC
cannot hot-add the input devices needed for Cua's background keyboard route,
so foreground input is used in this owned disposable desktop.

## Load the prototype in Roder

The following is an opt-in disposable evaluation configuration. Supply absolute
paths; the process host clears the child environment, so `HOME` is forwarded
for the Blaxel CLI's existing authenticated store.

```toml
[[process_extensions]]
id = "cua-linux-spike"
enabled = true
manifest = "/absolute/roder/examples/non-rust-extensions/cua-linux/roder-extension.toml"
command = "python3"
args = ["main.py"]
cwd = "/absolute/roder/examples/non-rust-extensions/cua-linux"
startup_timeout_ms = 10000
env = { HOME = "/absolute/home", RODER_CUA_SANDBOX = "OWNED-SANDBOX", RODER_CUA_WORKSPACE = "WORKSPACE", PYTHONUNBUFFERED = "1" }
```

Input is disabled by default. The scripted smoke sets
`RODER_CUA_ALLOW_INPUT=1` as the trusted evaluation launcher. Add it explicitly
only for a dedicated disposable evaluation: this prototype does **not** connect
input admission to Roder's per-turn policy mode, and must not be used as a
production or Plan-mode integration. The native contributor must implement that
policy boundary before general use. Each child binds to the first thread it
sees and refuses a second thread; production must resolve the runner from the
actual `ToolExecutionContext`.

The current Roder MCP adapter drops image content and uses HTTP rather than
Cua Driver's default stdio setup. This process extension demonstrates the
existing screenshot-preserving route without changing the core or inference
providers. It does not register the reserved OpenAI-native `computer` name.

## Checks and sources

```sh
python3 -m unittest discover -s examples/non-rust-extensions/cua-linux/tests
python3 -m py_compile examples/non-rust-extensions/cua-linux/*.py
```

- [Cua Driver platform support](https://cua.ai/docs/cua-driver/concepts/platform-support)
- [Pinned driver release](https://github.com/trycua/cua/releases/tag/cua-driver-rs-v0.34.0)
- [Blaxel desktop image source](https://github.com/blaxel-ai/sandbox/tree/main/hub/cua-xfce)
- [Blaxel image requirements](https://docs.blaxel.ai/Sandboxes/Templates)
