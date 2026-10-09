# Native desktop computer use with Cua

Enable the native desktop extension in Roder's config:

~~~toml
[cua]
enabled = true
backend = "runner"
program = "/opt/roder-cua/bin/cua-call"
timeout_ms = 45000
max_image_dimension = 1280
~~~

It is disabled by default. The program is a trusted absolute executable path
on the runner. Timeout is 1000–120000 ms; image dimension is 0 (native pixels)
or at most 4096. Unknown config keys are rejected.

Install the pinned driver/launcher in an owned graphical sandbox using
[the Linux fixture](../examples/cua-linux/README.md). A remote coding image
without a graphical session cannot run desktop tools. The tested profile is
Linux x86_64, XFCE/X11, AT-SPI and Cua Driver 0.34.0. The public Blaxel XFCE
template includes an older computer-server; this extension uses the separately
pinned driver and does not send requests to that server.

Bind the Roder thread to the provisioned sandbox through thread/start:

~~~json
{
  "workspaceId": "workspace-id",
  "runner": {
    "providerId": "blaxel",
    "workspace": "/home/cua",
    "config": {
      "sandbox_name": "owned-desktop",
      "image": "blaxel/cua-xfce:latest",
      "workspace": "your-blaxel-workspace",
      "working_dir": "/home/cua",
      "cleanup": "detach-on-close"
    }
  }
}
~~~

Supply credentials using the existing Blaxel runner environment variables.
No credentials belong in runner.config. A config-selected default runner
also binds new threads, including ACP sessions, when its provider supplies a
default workspace. For durable deployments, publish your provisioned desktop
image and pin its immutable version/digest in the destination config.

Build and launch applications with Roder's existing runner-bound coding tools.
Run GUI programs under the graphical session prefix established by the
installer, so their display and accessibility bus agree with the driver.
For example, this runner-side Python launches a compiled app:

~~~python
import json
from pathlib import Path
import subprocess

prefix = json.loads(Path("/opt/roder-cua/session.json").read_text())
subprocess.Popen(prefix + ["/home/cua/my-app"], start_new_session=True)
~~~

Discover the resulting native window with cua_list_windows, then observe and
operate it with desktop tools. The calculator fixture launches the app during
provisioning and advertises only desktop tools to the decision model for its
GUI acceptance task.

For a complete desktop workflow, the [full XFCE example](../examples/cua-linux/README.md#full-xfce-desktop-workflow)
installs Thunar and LibreOffice Writer and uses only desktop tools to launch
apps, navigate folders, drag a file and save/reopen an ODT document. Its
read-only grader verifies the resulting file bytes and document contents.
Use the [local live viewer](../examples/cua-linux/README.md#watch-the-desktop-live)
to watch the full screen while Roder works. The viewer sends no desktop input
and does not replace the agent's Cua grounding captures.

| Tools | Operation |
| --- | --- |
| cua_list_apps, cua_list_windows | Discover applications and exact native windows |
| cua_get_window_state | Window PNG, accessibility tree, capture and element handles |
| cua_get_desktop_state | Full desktop PNG and capture handle |
| cua_click | Element or pixel click; count 1–3; left/right/middle button |
| cua_drag, cua_scroll | Atomic drag; line/page scrolling |
| cua_press_key | Keys and chords to an exact window |
| cua_type_text | Linux ASCII/single-window insertion; macOS native targeted Unicode typing |
| cua_set_value | Exact editable replacement, including Unicode and dialogs |
| cua_move_cursor | Grounded full-desktop cursor movement |
| cua_bring_to_front, cua_set_window_frame | Explicit activation, movement and resizing |

## Browser control on the configured desktop

Enabling `[cua]` also makes the browser tools available to the agent. They use
Cua Driver on the same selected desktop and runner as native app tools. Roder's
native OpenAI `computer` adapter remains a separate Chrome integration; this
feature is the `cua_*` desktop contributor.

| Tool | Operation |
| --- | --- |
| cua_get_browser_state | Bind `pid`/`window_id`, then snapshot `target_id`/`tab_id`; semantic refs and an actual tab PNG |
| cua_browser_prepare | Launch an isolated driver profile or explicitly attach an existing profile |
| cua_browser_navigate | Navigate the exact bound tab; fresh semantic snapshot and PNG |
| cua_browser_click | Click a fresh ref declaring `click`; explicit trusted/DOM and background/foreground routes |
| cua_browser_type | Type Unicode into a fresh ref declaring `type`; optional replacement and keystrokes |
| cua_end_browser_session | Revoke this thread's browser capabilities and attachment; clean up throwaway profiles |

Discover the browser's native window first. Bind it with
`cua_get_browser_state({"pid":123,"window_id":456})`, retain the opaque
`target_id` and chosen `tab_id`, then snapshot with those two IDs. If `active`
is null, choose a tab explicitly from its URL/title. Mutations require an exact
binding. Query/scope/continuation are optional snapshot fields. Page text and
images are untrusted application content. Returned PNG coordinates describe
the tab viewport and do not authorize native-window pixel input.

Prepare accepts flat `profile_mode`: `isolated_new`, `isolated_named` (with
`profile_name`), or `existing_profile`. It requires an observed native browser
window. Isolated modes launch a separate driver-owned browser, so discover the
returned `prepared_pid` and bind its actual window. Existing-profile attachment
is **disabled by default**. Enable it only for a desktop/profile you authorize:

~~~toml
[cua]
enabled = true
backend = "runner"
allow_existing_browser_profile = true
~~~

The driver independently requires `--grant existing-profile` at daemon launch
or an equivalent approved bounded manifest. Roder configuration and ordinary
tool approval cannot provide that driver grant. The example's trusted
`/opt/roder-cua/driver-grants.json` contains `["existing-profile"]`; its launcher
reads the file only when starting the daemon. Install/change it as deployment
configuration, restart an existing daemon deliberately, and never pass a grant,
profile path, CDP endpoint or session ID as a model tool argument. On macOS,
launch the signed app-owned daemon with its own supported grant configuration.

Attachment exposes the approved browser's signed-in pages/storage. It uses the
running browser's consent/settings route and proves endpoint ownership; it does
not copy cookies or restart the profile. Do not copy a local personal profile
into a cloud sandbox. The X11 example signs into a local test website inside
its disposable sandbox and attaches that exact profile.

On Linux launch Chrome with `--force-renderer-accessibility` and
`ACCESSIBILITY_ENABLED=1`, under the same display and D-Bus session as Cua.
The fixture checks Chrome's native accessibility registration before the model
run. If automatic setup returns a refusal with visible side effects, inspect
the current native window before continuing; never blindly replay setup. An
explicitly authorized desktop agent can inspect Chrome's own remote-debugging
page through native Cua tools, then prepare/bind again. Preserve the refusal and
its side effects in the trace.

Use returned action refs for browser clicks/typing. Every successful snapshot
replaces the tab's refs, and native input or a browser mutation revokes cached
refs before dispatch. Refused/uncertain input is never retried automatically;
a fresh after-action snapshot is returned when available. Trusted background
clicks can refuse on Linux/macOS; `delivery_mode="foreground"` explicitly
permits activation on an owned desktop. `input_route="dom_event"` is synthetic
and requires fresh outcome verification. Browser controls without a supported
CDP route, browser chrome, dialogs, and file gestures can use native Cua input.

Finish with `cua_end_browser_session`. It deletes `isolated_new` profiles;
`isolated_named` and existing profiles persist. It revokes the driver attachment
but does not disable Chrome's remote-debugging setting; change that in Chrome
when needed. Roder process exit alone is not a claim that the browser's setting
was reset. Plan denies prepare/navigation/input/session cleanup; Default uses
the normal approval bridge. Browser inspection remains available.

The [browser example](../examples/cua-linux/README.md#browser-on-the-full-x11-desktop)
exercises this flow through the public Roder app-server and live model.
The upstream [browser guide](https://cua.ai/docs/cua-driver/guides/browsers#attach-to-a-logged-in-profile)
describes platform support, consent and refusals.

Window tools require pid and window_id from discovery. Observe the exact target
before input. Click/scroll requires either element_token or x, y and capture_id.
Drag takes from_x, from_y, to_x, to_y and capture_id. Cursor movement uses pixels
from the latest desktop capture. Handles are bound to the thread, runner
session and exact target. Stale handles, out-of-image coordinates and handles
from another thread fail before input. A fresh observation replaces previous
grounding. Rejoin or runtime restart requires another observation.

Use cua_set_value with an element_token to replace an editable's complete value;
this semantic tool has no delivery_mode argument.
On Linux, the pinned Cua 0.34 driver can truncate multibyte insertion and misaddress a
sibling window through its type_text route. Roder therefore rejects Unicode
type_text and typing into multi-window applications before input; exact value
replacement preserves the addressed window and supports full Unicode. Key
input remains available for applications without an editable accessibility node.

Element tokens and snapshot IDs are opaque Roder handles, bound to the thread,
runner and unique capture. Use their exact returned strings; raw Cua counter
tokens are rejected. Repeated counters after a driver restart cannot authorize
an old handle.

Pixels index the actual returned PNG; the driver maps scaled screenshots to
native input coordinates. Element handles preserve snapshot identity.
Background input is the default. Foreground delivery explicitly permits
activation. The extension never silently escalates or repeats refused or
uncertain input.

For a file move in Thunar, select the source icon before dragging and pass
`modifier: ["shift"]` to request a move explicitly. Ctrl requests a copy.
An input result with `effect: "unverifiable"` requires fresh app observation;
an immediate screenshot may precede an asynchronous file operation. After a
transport timeout or disconnection, observe rather than repeating the input.
The pinned driver may return a plain-text stale-window refusal. The Linux
launcher retains its message and error status so the agent can discover the
new window instead of losing the reason in a JSON decoding failure.

Plan mode permits observation and denies all desktop input, including scroll,
cursor movement and activation. Default mode requests Roder approval for each
input. Accept All and Bypass follow existing Roder policy modes. The executor
repeats the Plan check immediately before native dispatch.

Input is atomic; separate held-button tools are not exposed. Each input result
includes an after-action screenshot. A refused action remains an error even
when observation succeeds. Transport errors are treated as uncertain input,
followed by observation rather than retry. A failed capture revokes grounding
and returns an explicit error. A driver restart changes capture identities,
so old handles fail closed.

Desktop content is untrusted. PNG decoding validates format, dimensions,
metadata and byte/pixel bounds. Images use Roder's existing reserved display
payload. Responses, Anthropic and Gemini forward image blocks to the model.
ACP tool_call_update includes a native image block with the operation's text;
this adds tool output images, not ACP image prompt support.

Blaxel truncates process stdout at 64 KiB. The launcher writes the complete
bounded response into a host-selected result file and returns a tiny envelope.
Roder verifies the path, reads through its runner filesystem API and cleans up.
Dropping a command requests runner cancellation; input is never retried.
A bounded worker keeps the desktop fence until already-dispatched atomic
input finishes, even if its caller disappears. It discards abandoned results.
Later observations wait for this fence, preventing a cancelled drag's partial
state from authorizing new input before button release.

The runner remains the sandbox owner: pause/resume/detach/rejoin/close use its
existing lifecycle. Cua sessions derive from thread and runner IDs. The
extension does not persist credentials or target a process-global sandbox.

The calculator acceptance targets X11, satisfying the Linux windowing
requirement. Native Wayland requires separate compositor, capture, input and
AT-SPI qualification; an XFCE pass does not qualify it.
[Upstream platform support](https://cua.ai/docs/cua-driver/concepts/platform-support)
describes Sway, GNOME and experimental compositor routes.

## Local macOS

Select the local physical desktop explicitly; the default remains `runner`:

~~~toml
[cua]
enabled = true
backend = "local-macos"
timeout_ms = 45000
max_image_dimension = 1280
~~~

Use [the macOS fixture](../examples/cua-macos/README.md) to install the
checksum-verified signed **CuaDriver.app 0.34.0**, grant its Accessibility,
Screen Recording and direct capture access, and launch its daemon through
LaunchServices. Roder starts only finite `call` clients; it never starts a
terminal-owned daemon or grants permissions automatically. `program` defaults
to `/Applications/CuaDriver.app/Contents/MacOS/cua-driver`; `socket_path`
defaults to `$HOME/Library/Caches/cua-driver/cua-driver.sock` and must be an
absolute Unix socket path shorter than 104 bytes. `socket_path` is invalid
for `runner`. The client inherits only HOME/PATH/TMPDIR/LANG and disables
client telemetry; it does not inherit provider or runner credentials.

Create local threads without a runner binding. A local macOS contributor
refuses remote runner contexts, and the runner contributor refuses unbound
local contexts. Neither backend falls back to another desktop. The signed
app owns the permissions; Roder preserves its driver policy and tool refusals.

The same tools, approval checks and image payload work on both backends.
macOS native typing supports the qualified foreground Unicode/multiple-window
route. Background keys can refuse ambiguous windows; drag requires foreground.
Use pixel focus for text fields that do not expose AXPress. `cua_set_value`
is a background semantic value replacement, without a delivery-mode argument.
Modifier `super` means Command on macOS. Screenshots use actual PNG coordinates,
including Retina scaling and capped full-desktop cursor picks.

All local calls share a process-wide fence; input invalidates other local
threads' cached grounding, including across contributor instances. Cancellation
keeps the worker and fence alive for dispatched atomic input, and an
after-action observation waits for that worker. A lost/invalid response or
worker deadline blocks further calls until the operator restarts both the
app-owned daemon and Roder. A socket generation change rejects pre-restart
input and requires fresh observation. No mutation is automatically replayed.
Use one Roder process for local desktop automation: separate processes and
human input share the desktop and cannot be isolated like remote sandboxes.

Large accessibility trees retain capture/element handles in bounded valid
inline JSON. Full structured observations stay in the UI payload; omitted text
rows can be obtained with a narrower literal `query`. Images continue through
the existing provider and ACP paths. The macOS qualification records native
Calculator `42` via a live model and independent PNG OCR, plus an app-owned
primitive event/value grader. Apple Silicon macOS 26.6.2 is qualified here;
other macOS versions and Intel builds require their own validation.
