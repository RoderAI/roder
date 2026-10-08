# Native Linux computer use with Cua

Enable the native desktop extension in Roder's config:

~~~toml
[cua]
enabled = true
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

| Tools | Operation |
| --- | --- |
| cua_list_apps, cua_list_windows | Discover applications and exact native windows |
| cua_get_window_state | Window PNG, accessibility tree, capture and element handles |
| cua_get_desktop_state | Full desktop PNG and capture handle |
| cua_click | Element or pixel click; count 1–3; left/right/middle button |
| cua_drag, cua_scroll | Atomic drag; line/page scrolling |
| cua_press_key | Keys and chords to an exact window |
| cua_type_text | ASCII insertion into a single-window application |
| cua_set_value | Exact editable replacement, including Unicode and dialogs |
| cua_move_cursor | Grounded full-desktop cursor movement |
| cua_bring_to_front, cua_set_window_frame | Explicit activation, movement and resizing |

Window tools require pid and window_id from discovery. Observe the exact target
before input. Click/scroll requires either element_token or x, y and capture_id.
Drag takes from_x, from_y, to_x, to_y and capture_id. Cursor movement uses pixels
from the latest desktop capture. Handles are bound to the thread, runner
session and exact target. Stale handles, out-of-image coordinates and handles
from another thread fail before input. A fresh observation replaces previous
grounding. Rejoin or runtime restart requires another observation.

Use cua_set_value with an element_token to replace an editable's complete value.
The pinned Cua 0.34 driver can truncate multibyte insertion and misaddress a
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
describes Sway, GNOME and experimental compositor routes. Local macOS is an
optional future launcher, requiring its own Cua permissions.
