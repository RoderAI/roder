"""Validate complete attempt grids and regenerate all summary/paired artifacts."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent
PROVIDERS = "vision,jev-vision@jev-preview,jev_dom-vision@jev-preview"
SUITES = {"confirmation": (24, 3), "complex-dev": (8, 2),
          "complex-holdout": (6, 3), "search-regression": (4, 3)}
PAIRS = ["jev_dom-vision@jev-preview:jev-vision@jev-preview",
         "jev_dom-vision@jev-preview:vision", "jev-vision@jev-preview:vision"]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("suites", nargs="*", metavar="SUITE")
args = parser.parse_args()
wanted = args.suites or list(SUITES)
unknown = set(wanted) - SUITES.keys()
if unknown:
    parser.error(f"Unknown suites: {sorted(unknown)}")
for name in wanted:
    tasks, repeats = SUITES[name]
    source = ROOT / f"{name}.jsonl.gz"
    output = subprocess.check_output([sys.executable, str(ROOT.parent / "analyze.py"),
        "--tasks", str(tasks), "--repeats", str(repeats), "--providers", PROVIDERS,
        str(source)], text=True)
    summary = json.loads(output)
    (ROOT / f"{name}-summary.json").write_text(output)
    paired = subprocess.check_output([sys.executable, str(ROOT.parent / "paired.py"),
        str(source), *PAIRS], text=True)
    (ROOT / f"{name}-paired.json").write_text(paired)
    print(name, {p: f"{v['passed']}/{v['attempts']}" for p, v in summary.items()})
