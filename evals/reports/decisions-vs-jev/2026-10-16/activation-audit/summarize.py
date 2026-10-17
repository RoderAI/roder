"""Validate all frozen paired attempts and regenerate summaries."""
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
wanted = sys.argv[1:] or list(SUITES)
unknown = set(wanted) - SUITES.keys()
if unknown:
    raise SystemExit(f"Unknown suites: {sorted(unknown)}")
for name in wanted:
    tasks, repeats = SUITES[name]
    source = ROOT / f"{name}.jsonl.gz"
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
    (ROOT / f"{name}-summary.json").write_text(output)
    paired = subprocess.check_output([sys.executable, str(ROOT.parent / "paired.py"),
        str(source), *PAIRS], text=True)
    (ROOT / f"{name}-paired.json").write_text(paired)
    print(name, {p: f"{v['passed']}/{v['attempts']}" for p, v in summary.items()})
