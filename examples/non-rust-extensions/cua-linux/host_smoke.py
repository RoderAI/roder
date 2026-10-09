#!/usr/bin/env python3
"""Read-only check that an installed Roder app-server loads this extension."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sandbox", required=True)
    parser.add_argument("--workspace", required=True)
    parser.add_argument("--roder", default="roder")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    example = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix="roder-cua-host-") as temporary:
        config = Path(temporary) / "config"
        config.mkdir()
        forwarded = {"RODER_CUA_SANDBOX": args.sandbox, "RODER_CUA_WORKSPACE": args.workspace,
                     "HOME": os.environ["HOME"], "PYTHONUNBUFFERED": "1"}
        text = ('[[process_extensions]]\nid = "cua-linux-spike"\nenabled = true\n'
                + 'manifest = ' + json.dumps(str(example / "roder-extension.toml")) + '\n'
                + 'command = "python3"\nargs = ["main.py"]\ncwd = ' + json.dumps(str(example)) + '\n'
                + 'startup_timeout_ms = 10000\nenv = {'
                + ', '.join(k + ' = ' + json.dumps(v) for k, v in forwarded.items()) + '}\n')
        (config / "config.toml").write_text(text)
        requests = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize",
             "params": {"clientInfo": {"name": "cua-eval", "version": "0.1"}}},
            {"jsonrpc": "2.0", "id": 2, "method": "extensions/list", "params": {}},
        ]
        process = subprocess.run(
            [args.roder, "app-server"], input=''.join(json.dumps(r) + '\n' for r in requests),
            capture_output=True, text=True, timeout=30,
            env={**os.environ, "RODER_CONFIG_DIR": str(config), "RODER_DATA_DIR": str(Path(temporary) / "data")},
        )
        if process.returncode:
            raise RuntimeError("Roder app-server failed to start")
        response = next(json.loads(line) for line in process.stdout.splitlines() if json.loads(line).get("id") == 2)
        match = next(e for e in response["result"]["extensions"] if e["id"] == "cua-linux-spike")
        report = {"passed": True, "extension": match,
                  "scope": "Installed Roder app-server manifest loading and extension initialization only; no model turn"}
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({"passed": True, "extension_id": match["id"], "scope": report["scope"]}))


if __name__ == "__main__":
    main()
