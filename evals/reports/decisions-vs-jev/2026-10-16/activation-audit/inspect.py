"""Check image placement and retain actual model IDs and reference counts."""
import argparse
import collections
import gzip
import json
from pathlib import Path
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("sources", nargs="+", type=Path)
args = parser.parse_args()
if len({source.name for source in args.sources}) != len(args.sources):
    parser.error("Source basenames must be unique; inspect matching names separately")

def require(condition, message):
    if not condition:
        raise ValueError(message)

result = {}
for source in args.sources:
    content = gzip.open(source, "rt").read() if source.suffix == ".gz" else source.read_text()
    groups = collections.defaultdict(lambda: {"wire_calls": 0, "models": collections.Counter(),
        "image_placement": collections.Counter(), "image_references": 0, "wire_errors": 0, "activation_audits": 0})
    for line in content.splitlines():
        row = json.loads(line)
        provider = row["telemetry"]["provider"]
        group = groups[provider]
        for wire in row["telemetry"].get("wire", []):
            request, response = wire["request"], wire.get("response") or {}
            group["wire_calls"] += 1
            group["models"][response.get("model") or "unknown"] += 1
            group["wire_errors"] += bool(wire.get("error"))
            if provider.startswith("jev") and set(request.get("questions", {})) == {"required", "performed", "ready"}:
                require("jev_activation" in provider, "Audit from unexpected provider")
                require(not request.get("assets"), "Audit unexpectedly includes images")
                require(all(q["type"] == "noul" for q in request["questions"].values()), "Audit question type")
                group["activation_audits"] += 1
            if not provider.startswith("jev"):
                continue
            assets = request.get("assets") or {}
            if not assets:
                group["image_placement"]["text"] += 1
                continue
            require(len(assets) == 1, "Expected one fitted viewport asset")
            reference = next(iter(assets))
            state_count = json.dumps(request["state"]).count(reference)
            question_counts = {name: json.dumps(question).count(reference)
                               for name, question in request["questions"].items()}
            references = state_count + sum(question_counts.values())
            group["image_references"] += references
            if state_count:
                require(state_count == 1 and not any(question_counts.values()), "Mixed image placement")
                group["image_placement"]["shared"] += 1
            else:
                require("jev_dom_targets" in provider or "jev_activation" in provider, "Unexpected scoped-image provider")
                require(references > 0, "Unreferenced image asset")
                require(all(not count or (name.endswith("_target") and
                    request["questions"][name]["type"] == "choice")
                    for name, count in question_counts.items()), "Image outside a target Choice")
                require(any("TYPE_TEXT" in e["operations"] and e.get("value")
                           for e in request["state"]["elements"]), "Scoped image without a populated field")
                group["image_placement"]["targets"] += 1
    result[source.name] = groups
print(json.dumps(result, indent=2))
