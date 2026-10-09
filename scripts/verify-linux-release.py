#!/usr/bin/env python3
"""Check the shipped Linux ABI and startup before publishing an archive."""
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import sys
import tempfile
import threading


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def request(child, messages, identifier, method, params):
    child.stdin.write(json.dumps({"jsonrpc": "2.0", "id": identifier,
                                 "method": method, "params": params}) + "\n")
    child.stdin.flush()
    while True:
        reply = messages.get(timeout=30)
        if reply.get("id") == identifier:
            if "error" in reply:
                raise RuntimeError(f"{method} failed: {reply['error']['code']}")
            return reply["result"]


def smoke(binary, mode, config, data, expected_version):
    child = subprocess.Popen([str(binary), mode], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             text=True, env={**os.environ,
                             "RODER_CONFIG_DIR": str(config), "RODER_DATA_DIR": str(data)})
    messages = queue.Queue()
    diagnostics = []

    def read():
        for line in child.stdout:
            try:
                messages.put(json.loads(line))
            except ValueError:
                pass

    def read_errors():
        for line in child.stderr:
            diagnostics.append(line)
            del diagnostics[:-20]

    threading.Thread(target=read, daemon=True).start()
    threading.Thread(target=read_errors, daemon=True).start()
    try:
        client = {"name": "linux-release-verification", "version": "1"}
        params = {"clientInfo": client}
        if mode == "acp":
            params.update(protocolVersion=1, clientCapabilities={})
        reply = request(child, messages, 1, "initialize", params)
        if mode == "acp":
            require(reply["agentInfo"]["version"] == expected_version,
                    "ACP version does not match the release manifest")
            return {"mode": mode, "agentInfo": reply["agentInfo"]}
        tools = request(child, messages, 2, "tools/list", {})["tools"]
        names = sorted(t["name"] for t in tools if t["name"].startswith("cua_"))
        enabled = "enabled = true" in (config / "config.toml").read_text()
        if enabled:
            require(len(names) >= 19 and {"cua_get_desktop_state", "cua_browser_type",
                                         "cua_end_browser_session"}.issubset(names),
                    f"Cua tools missing from enabled release: {names}")
        else:
            require(not names, f"Cua tools exposed without opt-in: {names}")
        return {"mode": mode, "cua_enabled": enabled, "cua_tools": names}
    except (BrokenPipeError, queue.Empty) as error:
        raise RuntimeError("Release binary did not answer initialize: " +
                           "".join(diagnostics)) from error
    finally:
        try:
            child.stdin.close()
        except BrokenPipeError:
            pass
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=10)


def main():
    binary = Path(sys.argv[1]).resolve()
    target = sys.argv[2]
    require(target in {"x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"},
            f"Unsupported Linux release target: {target}")
    symbols = subprocess.check_output(["readelf", "--version-info", str(binary)], text=True)
    versions = {tuple(map(int, v.split("."))) for v in re.findall(r"GLIBC_(\d+\.\d+(?:\.\d+)?)", symbols)}
    require(versions, "No GLIBC version requirements found")
    highest = max(versions)
    require(highest <= (2, 35),
            f"{target} requires GLIBC {highest}; Ubuntu 22.04 supplies 2.35")
    require("GLIBC_ABI_DT_RELR" not in symbols, "DT_RELR requires a newer GLIBC loader")
    manifest = (Path(__file__).resolve().parent.parent / "crates/roder-app-server/Cargo.toml").read_text()
    expected = re.search(r'^version\s*=\s*"([^"]+)"', manifest, re.MULTILINE).group(1)
    checks = []
    with tempfile.TemporaryDirectory(prefix="roder-linux-release-") as directory:
        root = Path(directory)
        for enabled in [False, True]:
            config = root / ("enabled" if enabled else "disabled")
            config.mkdir()
            (config / "config.toml").write_text('[cua]\nenabled = ' +
                                               str(enabled).lower() + '\nbackend = "runner"\n')
            checks.append(smoke(binary, "app-server", config, root / "data", expected))
            if enabled:
                checks.append(smoke(binary, "acp", config, root / "acp-data", expected))
    print(json.dumps({"target": target, "max_glibc": ".".join(map(str, highest)),
                      "startup_checks": checks, "passed": True}))


if __name__ == "__main__":
    main()
