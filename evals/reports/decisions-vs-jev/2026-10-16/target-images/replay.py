"""Replay paid TypeSafe development probes from their retained exact requests.

Requires Python 3.9+ and JEV_API_KEY or TYPESAFE_API_KEY. These judgments
are state replays, not independent browser attempts or outcome grades.
"""
import argparse
import gzip
import json
import os
from pathlib import Path
import time
import urllib.request
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("source", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--variant")
args = parser.parse_args()
key = os.getenv("JEV_API_KEY") or os.getenv("TYPESAFE_API_KEY")
assert key, "Set a TypeSafe API key"
assert not args.output.exists(), "Output must be a new file"
with gzip.open(args.source, "rt") as source, args.output.open("x") as output:
    for line in source:
        original = json.loads(line)
        if args.variant and original["variant"] != args.variant:
            continue
        headers = {"Authorization": "Bearer " + key, "Content-Type": "application/json"}
        if original["request"].get("assets"):
            headers["TypeSafe-Danger-Zone-Images"] = "true"
        request = urllib.request.Request("https://api.typesafe.ai/v1/systemone",
            data=json.dumps(original["request"]).encode(), headers=headers)
        started = time.monotonic()
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                original["response"] = json.load(response)
            original["error"] = None
        except Exception as error:
            original["response"] = None
            original["error"] = str(error)
        original["latency_ms"] = round((time.monotonic() - started) * 1000)
        output.write(json.dumps(original) + "\n")
        output.flush()
        print(original["variant"], original["repeat"], original["source_step"],
              (original["response"] or {}).get("answers"), flush=True)
