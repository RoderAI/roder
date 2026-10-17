"""Validate all frozen paired attempts and regenerate summaries."""
import argparse
import gzip
import json
from pathlib import Path
import subprocess
import sys
ROOT = Path(__file__).resolve().parent
GRID = json.loads((ROOT / "task-grid.json").read_text())
PROVIDERS = "vision,jev_dom_targets-vision@jev-1.14.0,jev_activation-vision@jev-1.14.0"
SUITES = {"confirmation": (24, 3), "complex-dev": (8, 2),
          "compound-regression": (6, 3), "search-regression": (4, 3), "transfer": (8, 3), "completion-regression": (4, 3), "completion-transfer": (4, 3)}
PAIRS = ["jev_activation-vision@jev-1.14.0:jev_dom_targets-vision@jev-1.14.0",
         "jev_activation-vision@jev-1.14.0:vision", "jev_dom_targets-vision@jev-1.14.0:vision"]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--data-directory', type=Path, default=ROOT)
parser.add_argument('suites', nargs='*')
args = parser.parse_args()
wanted = args.suites or list(SUITES)
unknown = set(wanted) - SUITES.keys()
if unknown:
    raise SystemExit(f"Unknown suites: {sorted(unknown)}")
for name in wanted:
    tasks, repeats = SUITES[name]
    source = args.data_directory / f"{name}.jsonl.gz"
    rows = [json.loads(line) for line in gzip.open(source, 'rt') if line.strip()]
    keys = [(r['task'], r['repeat'], r['telemetry']['provider']) for r in rows]
    expected = {(task, repeat, provider) for task in GRID[name]
                for repeat in range(1, repeats + 1) for provider in PROVIDERS.split(',')}
    if len(keys) != len(set(keys)) or set(keys) != expected:
        raise SystemExit('Duplicate, missing or non-frozen attempt: ' + name)
    output = subprocess.check_output([sys.executable, str(ROOT.parent / "analyze.py"),
        "--tasks", str(tasks), "--repeats", str(repeats), "--providers", PROVIDERS,
        str(source)], text=True)
    summary = json.loads(output)
    (args.data_directory / f"{name}-summary.json").write_text(output)
    paired = subprocess.check_output([sys.executable, str(ROOT.parent / "paired.py"),
        str(source), *PAIRS], text=True)
    (args.data_directory / f"{name}-paired.json").write_text(paired)
    print(name, {p: f"{v['passed']}/{v['attempts']}" for p, v in summary.items()})
