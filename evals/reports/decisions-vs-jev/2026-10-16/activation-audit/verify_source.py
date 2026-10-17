"""Verify the frozen source hashes against their recorded Git commit."""
import hashlib
import json
from pathlib import Path
import subprocess
ROOT = Path(__file__).resolve().parent
freeze = json.loads((ROOT / "freeze.json").read_text())
commit = freeze["code_commit"]
if hashlib.sha256((ROOT / "protocol.json").read_bytes()).hexdigest() != freeze["protocol_sha256"]:
    raise SystemExit("Protocol hash mismatch")
for name, expected in freeze["source_sha256"].items():
    content = subprocess.check_output(["git", "show", commit + ":" + name], cwd=ROOT)
    if hashlib.sha256(content).hexdigest() != expected:
        raise SystemExit(f"Source hash mismatch: {name}")
print(f"Verified {len(freeze['source_sha256'])} source files at {commit}")
