# Agentic Trace Fixtures

This directory stores compact local audit fixtures for bt-gbrain agentic trace
work. The fixture data is derived from JSON files under
`/Users/pz/w/OrgMemBench/results` and source inspection in
`crates/roder-ext-gbrain`; it does not require live provider calls.

`current-runs.json` records:

- completed OrgMemBench metrics for every local `helix-*.json` result file
- trace-shape facts, including fixed `llm_calls` and fixed subquery counts
- source findings showing that the current provider path sends no tools and the
  agentic retriever is still fake-planner-only
- representative hallucination rows with multiple verified claims
- representative retrieval-miss or wrong-abstain rows with large evidence pools

The fixture is intentionally compact. Use the full result JSON files when a
future task needs answer text, judge rationale, or full evidence payloads.
