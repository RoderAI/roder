#!/usr/bin/env python3
"""Live native calculator test through the example's JSON-RPC child protocol."""

from __future__ import annotations

import argparse
import base64
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import select
import subprocess

from transport import BlaxelTransport


class Child:
    def __init__(self, sandbox, workspace):
        self.process = subprocess.Popen(
            ["python3", str(Path(__file__).with_name("main.py"))],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
            env={**os.environ, "RODER_CUA_SANDBOX": sandbox, "RODER_CUA_WORKSPACE": workspace,
                 "RODER_CUA_ALLOW_INPUT": "1"},
        )
        self.next_id = 0
        self.trace = []
        self.request("extension/initialize", {})

    def request(self, method, params):
        self.next_id += 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.next_id,
                                            "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        if not select.select([self.process.stdout], [], [], 80)[0]:
            raise RuntimeError("Extension timeout; do not replay an uncertain input")
        response = json.loads(self.process.stdout.readline())
        if "error" in response:
            raise RuntimeError(response["error"]["message"])
        return response["result"]

    def tool(self, name, arguments, allow_refusal=False):
        result = self.request("tools/call", {"providerId": "cua-linux-spike", "toolName": name,
                             "threadId": "cua-calculator-eval", "turnId": "scripted-spike",
                             "callId": f"call-{self.next_id + 1}", "arguments": arguments})
        self.trace.append({"tool": name, "arguments": arguments,
                           "is_error": result["isError"], "observation": result["data"]["observation"],
                           "image_returned": "__view_image" in result["data"]})
        if result["isError"] and not allow_refusal:
            raise RuntimeError("Cua refused action: " + result["content"][:1000])
        return result

    def close(self):
        if self.process.poll() is None:
            try:
                self.request("extension/shutdown", {})
                self.process.wait(timeout=5)
            finally:
                if self.process.poll() is None:
                    self.process.kill()
                    self.process.wait()


def capture(child, window, out, name):
    result = child.tool("cua_get_window_state", {**window, "max_image_dimension": 0})
    state = result["data"]["observation"]
    (out / f"{name}.json").write_text(json.dumps(state, indent=2) + "\n")
    encoded = result["data"]["__view_image"]["image_url"].split(",", 1)[1]
    (out / f"{name}.png").write_bytes(base64.b64decode(encoded, validate=True))
    return state


def independent_grader(transport, pid):
    # Read app-owned AT-SPI state independently of Cua's action responses and
    # snapshot formatter. The grader never mutates or sets calculator state.
    reader = f'''import gi,json
gi.require_version("Atspi","2.0")
from gi.repository import Atspi
values=[]
def visit(node,depth=0):
 if depth>15:return
 try:
  if node.get_role_name()=="text":
   text=node.get_text_iface()
   values.append(text.get_text(0,text.get_character_count()) if text else node.get_name())
  for index in range(node.get_child_count()):visit(node.get_child_at_index(index),depth+1)
 except Exception:pass
desktop=Atspi.get_desktop(0)
for index in range(desktop.get_child_count()):
 app=desktop.get_child_at_index(index)
 if app.get_process_id()=={pid}:visit(app)
print(json.dumps({{"display_values":values,"passed":"42" in values}}))
'''
    script = ("import json,pathlib,subprocess,sys\n"
              "prefix=json.loads(pathlib.Path('/tmp/roder-cua-session.json').read_text())\n"
              f"p=subprocess.run(prefix+['/usr/bin/python3','-c',{reader!r}],capture_output=True,text=True,timeout=30)\n"
              "print(p.stdout);print(p.stderr,file=sys.stderr);raise SystemExit(p.returncode)\n")
    result = transport.run_script(script)
    if result.get("exitCode") != 0:
        raise RuntimeError("Independent AT-SPI grader failed: " + result.get("stderr", "")[-500:])
    return json.loads(result["stdout"])


def run(sandbox, workspace, out):
    out.mkdir(parents=True, exist_ok=True)
    child = Child(sandbox, workspace)
    report = {"timestamp": datetime.now(timezone.utc).isoformat(), "sandbox": sandbox,
              "workspace": workspace, "backend": "Blaxel Linux XFCE/X11", "driver_version": "0.34.0",
              "mode": "scripted process-extension JSON-RPC", "roder_host_exercised": False,
              "live_model_exercised": False, "native_wayland_exercised": False, "passed": False}
    try:
        windows = child.tool("cua_list_windows", {})["data"]["observation"]["windows"]
        calc = next(w for w in windows if w["app_name"].lower() == "galculator")
        window = {"pid": calc["pid"], "window_id": calc["window_id"]}
        for i, label in enumerate(["AC", "6", "*", "7", "="]):
            state = capture(child, window, out, f"step-{i}")
            element = next(e for e in state["elements"] if e.get("label") == label and "click" in e.get("actions", []))
            if label == "7":
                result = child.tool("cua_press_key", {**window, "key": "7"}, allow_refusal=True)
                if result["isError"]:
                    if result["data"]["observation"].get("code") != "background_unavailable":
                        raise RuntimeError("Unexpected input failure; no automatic retry")
                    # The driver explicitly refused before delivery. This is
                    # an owned disposable desktop, so foreground is permitted.
                    child.tool("cua_press_key", {**window, "key": "7", "delivery_mode": "foreground"})
            elif label in ("6", "="):
                frame = element["screenshot_frame"]
                child.tool("cua_click", {**window, "x": frame["x"] + frame["w"] / 2,
                           "y": frame["y"] + frame["h"] / 2, "capture_id": state["capture_id"]})
            else:
                child.tool("cua_click", {**window, "element_token": element["element_token"]})
        final = capture(child, window, out, "final")
        report.update(window=window,
                      displayed_values=[e.get("value", e.get("label")) for e in final["elements"] if e.get("role") == "text"])
        desktop = child.tool("cua_get_desktop_state", {"max_image_dimension": 0})
        (out / "desktop.png").write_bytes(base64.b64decode(desktop["data"]["__view_image"]["image_url"].split(",", 1)[1]))
        grader = independent_grader(BlaxelTransport(sandbox, workspace), window["pid"])
        report.update(independent_grader=grader, passed=grader["passed"], trace=child.trace)
        if not report["passed"]:
            raise RuntimeError("Calculator did not display 42")
    except Exception as error:
        report.update(error=str(error), trace=child.trace)
        raise
    finally:
        (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        child.close()
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sandbox", required=True)
    parser.add_argument("--workspace", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = run(args.sandbox, args.workspace, args.output)
    print(json.dumps({k: report[k] for k in ("passed", "backend", "mode", "independent_grader")}))


if __name__ == "__main__":
    main()
