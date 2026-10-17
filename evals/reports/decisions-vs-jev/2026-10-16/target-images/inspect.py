"""Check image placement and retain actual model IDs and reference counts."""
import argparse
import collections
import gzip
import json
from pathlib import Path
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("sources", nargs="+", type=Path)
args = parser.parse_args()
result = {}
for source in args.sources:
    content = gzip.open(source, "rt").read() if source.suffix == ".gz" else source.read_text()
    groups = collections.defaultdict(lambda: {"wire_calls": 0, "models": collections.Counter(),
        "image_placement": collections.Counter(), "image_references": 0, "wire_errors": 0})
    for line in content.splitlines():
        row = json.loads(line)
        provider = row["telemetry"]["provider"]
        group = groups[provider]
        for wire in row["telemetry"].get("wire", []):
            request, response = wire["request"], wire.get("response") or {}
            group["wire_calls"] += 1
            group["models"][response.get("model") or "unknown"] += 1
            group["wire_errors"] += bool(wire.get("error"))
            if not provider.startswith("jev"):
                continue
            assets = request.get("assets") or {}
            if not assets:
                group["image_placement"]["text"] += 1
                continue
            assert len(assets) == 1
            reference = next(iter(assets))
            state_count = json.dumps(request["state"]).count(reference)
            question_counts = {name: json.dumps(question).count(reference)
                               for name, question in request["questions"].items()}
            references = state_count + sum(question_counts.values())
            group["image_references"] += references
            if state_count:
                assert state_count == 1 and not any(question_counts.values())
                group["image_placement"]["shared"] += 1
            else:
                assert "jev_dom_targets" in provider
                assert references > 0
                assert all(not count or (name.endswith("_target") and
                    request["questions"][name]["type"] == "choice")
                    for name, count in question_counts.items())
                assert any("TYPE_TEXT" in e["operations"] and e.get("value")
                           for e in request["state"]["elements"])
                group["image_placement"]["targets"] += 1
    result[source.name] = groups
print(json.dumps(result, indent=2))
