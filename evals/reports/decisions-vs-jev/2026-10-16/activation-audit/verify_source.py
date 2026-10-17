"""Verify the frozen source hashes against their recorded Git commit."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
ROOT = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--working-tree', action='store_true', help='Also require frozen source bytes in the current checkout')
parser.add_argument('--source-checkout', type=Path, help='Frozen build checkout whose source bytes and fixtures must match')
args = parser.parse_args()
checkout = Path(subprocess.check_output(['git','rev-parse','--show-toplevel'], cwd=ROOT, text=True).strip())
if args.source_checkout:
    checkout = args.source_checkout.resolve()
    args.working_tree = True
freeze = json.loads((ROOT / "freeze.json").read_text())
commit = freeze["code_commit"]
if hashlib.sha256((ROOT / "protocol.json").read_bytes()).hexdigest() != freeze["protocol_sha256"]:
    raise SystemExit("Protocol hash mismatch")
for name, expected in freeze["source_sha256"].items():
    content = subprocess.check_output(["git", "show", commit + ":" + name], cwd=ROOT)
    if hashlib.sha256(content).hexdigest() != expected:
        raise SystemExit(f"Source hash mismatch: {name}")
    if args.working_tree and hashlib.sha256((checkout / name).read_bytes()).hexdigest() != expected:
        raise SystemExit(f"Working-tree source mismatch: {name}")
print(f"Verified {len(freeze['source_sha256'])} source files at {commit}")
