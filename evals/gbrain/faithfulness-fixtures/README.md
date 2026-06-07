# bt-gbrain faithfulness fixtures

These local fixtures capture the residual failure classes seen in the OrgMemBench
medium runs and PR #30 comments. They are deliberately small and deterministic:
the expected result is a ledger or grounding decision, not an LLM score.

The initial strict validator covers the first five cases directly:

- quote spans are mandatory
- unsupported dates, numbers, artifact ids, and named entities are rejected
- direct claims cannot blend multiple records
- since-as-of claims need explicit change/replacement/unchanged support
- missing artifact ids are rejected

The remaining cases document the next validation hooks: source-type aware
direct/inferred classification, conflict completeness, and one-event evidence
chain enforcement.
