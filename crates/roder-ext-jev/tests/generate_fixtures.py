"""Record upstream Jev's behaviour as fixtures for the Rust port's parity tests.

Run against the pinned upstream revision:

    uv run --no-project \
      --with "jev-ultrafast @ git+https://github.com/browser-use/jev-ultrafast.git@1231850a0bf1a0c0341fe408ef1668dbbfdfac46" \
      python crates/roder-ext-jev/tests/generate_fixtures.py

Every file it writes is upstream output, not hand-written expectation: the Rust
implementation is correct when it reproduces these byte for byte.

Jev now owns the fixtures and diverges in places this script cannot record, so
reapply them after regenerating: TARGET's closing sentences about offscreen
elements and `context` (prompts.json, and the rules in choose_request.json's body and
serialized form), the offscreen "Next page" link in action_space.json, and the
`date` key after `goal` in field_context.json's context (recorded as of
2026-09-27), the `other_fields` key after `field` there (the fixture page's
"Cabin" select), and both in the user message of every
field_text_requests.json body.
"""

import hashlib
import json
import os
import pathlib
import sys

os.environ.setdefault("TYPESAFE_MODEL", "jev-latest")
os.environ.setdefault("TYPESAFE_API_KEY", "fixture-key")

from jev_ultrafast import browser, model  # noqa: E402

OUT = pathlib.Path(__file__).parent / "fixtures"

# One observation exercising every branch of the action space: a plain button, an
# editable field (which also yields an "Open ..." click), a multi-option select
# whose labels carry the " → " separator, a second action on an already-indexed
# node, aria state, and the synthetic scroll/wait controls.
ACTIONS = [
    {"id": "e1", "node": 11, "kind": "click", "role": "button", "label": "Search",
     "value": "", "rect": {"x": 10.0, "y": 20.5, "w": 80.0, "h": 24.0}},
    {"id": "e2", "node": 12, "kind": "fill", "role": "searchbox", "label": "Where to?",
     "value": "Zur", "rect": {"x": 10.0, "y": 60.0, "w": 200.0, "h": 32.0}},
    {"id": "e3", "node": 12, "kind": "click", "role": "searchbox", "label": "Open Where to?",
     "value": "Zur", "rect": {"x": 10.0, "y": 60.0, "w": 200.0, "h": 32.0}},
    {"id": "e4", "node": 13, "kind": "select", "role": "combobox", "label": "Cabin → Economy",
     "value": "economy", "current_value": "Any", "rect": {"x": 10.0, "y": 100.0, "w": 120.0, "h": 28.0}},
    {"id": "e5", "node": 13, "kind": "select", "role": "combobox", "label": "Cabin → Business",
     "value": "business", "current_value": "Any", "rect": {"x": 10.0, "y": 100.0, "w": 120.0, "h": 28.0}},
    {"id": "e6", "node": 14, "kind": "click", "role": "checkbox", "label": "Nonstop only",
     "value": "on", "checked": "false", "rect": {"x": 10.0, "y": 140.0, "w": 16.0, "h": 16.0}},
    {"id": "e7", "node": 15, "kind": "click", "role": "tab", "label": "Cheapest",
     "value": "", "selected": "false", "expanded": "true",
     "rect": {"x": 10.0, "y": 180.0, "w": 90.0, "h": 30.0}},
    {"id": "scroll_down", "kind": "scroll", "label": "Scroll down", "delta": 560},
    {"id": "wait", "kind": "wait", "label": "Wait for the page to update"},
]

PAGE = {
    "url": "https://example.test/search?q=1",
    "title": "Search — Example",
    "text": "Search\nWhere to?\nCabin\nNonstop only\nCheapest\nPrices in £",
    "w": 1120,
    "h": 780,
    "scroll": {"y": 0, "height": 2400},
    "actions": ACTIONS,
    "guards": {"11": [11, "button", "Search"], "12": [12, "searchbox", "Where to?"]},
    "marker": [12345.0, "https://example.test/search?q=1", 0, 0, 1120, 780],
    "page_key": [12345.0, "https://example.test/search?q=1", 0, 0, 1120, 780, []],
    "omitted_actions": 0,
}

HISTORY = [
    {"step": 1, "action": "Accept cookies", "kind": "click", "text": None, "page_changed": True,
     "choice": "e9", "url": "https://example.test/"},
    {"step": 2, "action": "Where to?", "kind": "fill", "text": "Zurich", "page_changed": False,
     "choice": "e2", "url": "https://example.test/search?q=1"},
]

GOAL = "Find one-way flights from Zurich to London on 2026-09-28 in economy."


def jsonable(value):
    """Fixtures are compared against Rust, so keys stay strings as JSON requires."""
    if isinstance(value, dict):
        return {str(k): jsonable(v) for k, v in value.items()}
    if isinstance(value, list):
        return [jsonable(v) for v in value]
    return value


def write(name, value):
    path = OUT / name
    path.write_text(json.dumps(jsonable(value), indent=2, ensure_ascii=False) + "\n")
    print("wrote", path.name)


def capture_body(call):
    """Run a model call with the network replaced, keeping the request body."""
    captured = {}
    original = model.post_json

    def fake(url, key, body):
        captured["url"] = url
        captured["key"] = key
        captured["body"] = body
        raise RuntimeError("captured")

    model.post_json = fake
    try:
        call()
    except RuntimeError as error:
        if str(error) != "captured":
            raise
    finally:
        model.post_json = original
    return captured


def main():
    OUT.mkdir(parents=True, exist_ok=True)

    elements, targets, controls = model.action_space(ACTIONS)
    write("action_space.json", {
        "input_actions": ACTIONS,
        "elements": elements,
        "targets": {op: {target: action["id"] for target, action in group.items()}
                    for op, group in targets.items()},
        "target_order": {op: list(group) for op, group in targets.items()},
        "controls": {name: action["id"] for name, action in controls.items()},
    })

    captured = capture_body(lambda: model.choose(PAGE, GOAL, HISTORY))
    write("choose_request.json", {
        "url": captured["url"],
        "goal": GOAL,
        "history": HISTORY,
        "page": {k: PAGE[k] for k in ("url", "title", "text")},
        "body": captured["body"],
        # httpx serializes with compact separators; the Rust port must match this
        # byte for byte, which also pins key order.
        "serialized": json.dumps(captured["body"], separators=(",", ":"), ensure_ascii=False),
    })

    fill_action = ACTIONS[1]
    context = model.field_context(GOAL, fill_action, PAGE, HISTORY)
    write("field_context.json", {"goal": GOAL, "action": fill_action, "context": context})

    text_bodies = {}
    for name, base in {
        "deepseek": "https://api.deepseek.com/v1",
        "openrouter": "https://openrouter.ai/api/v1",
        "other": "https://api.example.test/v1",
    }.items():
        for reasoning_env in (None, "none"):
            os.environ["TEXT_MODEL_API_KEY"] = "fixture-text-key"
            os.environ["TEXT_MODEL_BASE_URL"] = base
            os.environ["TEXT_MODEL"] = "fixture-writer"
            os.environ.pop("TEXT_MODEL_REASONING", None)
            if reasoning_env:
                os.environ["TEXT_MODEL_REASONING"] = reasoning_env
            captured = capture_body(lambda: model.field_text(context))
            key = f"{name}_{reasoning_env or 'default'}"
            text_bodies[key] = {"url": captured["url"], "body": captured["body"]}
    write("field_text_requests.json", text_bodies)

    # Upstream hashes a canonical dump of exactly these four fields.
    content = {k: PAGE[k] for k in ("url", "text", "actions", "scroll")}
    write("fingerprint.json", {
        "page": PAGE,
        "canonical": json.dumps(content, sort_keys=True),
        "sha256": hashlib.sha256(json.dumps(content, sort_keys=True).encode()).hexdigest(),
        "upstream": browser.fingerprint(PAGE),
    })

    ids = {"e1": {}, "e2": {}, "DONE": {}}
    cases = [
        {"name": "valid", "answer": {"choice": "e1", "confidence": 0.9,
                                    "probabilities": {"e1": 0.8, "e2": 0.15, "DONE": 0.05}}},
        {"name": "unknown_choice", "answer": {"choice": "e9", "confidence": 0.9,
                                              "probabilities": {"e1": 0.8, "e2": 0.15, "DONE": 0.05}}},
        {"name": "missing_probability", "answer": {"choice": "e1", "confidence": 0.9,
                                                   "probabilities": {"e1": 0.9, "e2": 0.1}}},
        {"name": "sum_out_of_tolerance", "answer": {"choice": "e1", "confidence": 0.5,
                                                    "probabilities": {"e1": 0.5, "e2": 0.2, "DONE": 0.2}}},
        {"name": "sum_within_tolerance", "answer": {"choice": "e1", "confidence": 0.5,
                                                    "probabilities": {"e1": 0.5, "e2": 0.3, "DONE": 0.19}}},
        {"name": "choice_not_argmax", "answer": {"choice": "e2", "confidence": 0.9,
                                                 "probabilities": {"e1": 0.8, "e2": 0.15, "DONE": 0.05}}},
        {"name": "confidence_out_of_range", "answer": {"choice": "e1", "confidence": 1.4,
                                                       "probabilities": {"e1": 0.8, "e2": 0.15, "DONE": 0.05}}},
        {"name": "probability_not_a_number", "answer": {"choice": "e1", "confidence": 0.9,
                                                        "probabilities": {"e1": "high", "e2": 0.05, "DONE": 0.05}}},
        {"name": "missing_confidence", "answer": {"choice": "e1",
                                                  "probabilities": {"e1": 0.8, "e2": 0.15, "DONE": 0.05}}},
        {"name": "empty", "answer": {}},
    ]
    for case in cases:
        try:
            model.validate_choice(case["answer"], ids)
            case["accepted"] = True
        except ValueError:
            case["accepted"] = False
    write("validate_choice.json", {"ids": list(ids), "cases": cases})

    write("prompts.json", {
        "NEXT_ACTION": model.NEXT_ACTION,
        "TARGET": model.TARGET,
        "TEXT_VALUE": model.TEXT_VALUE,
        "MAX_STEPS": __import__("jev_ultrafast.questions", fromlist=["MAX_STEPS"]).MAX_STEPS,
    })
    print("jev version pinned in fixtures:", sys.argv[0])


if __name__ == "__main__":
    main()
