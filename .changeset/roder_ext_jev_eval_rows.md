---
roder-ext-jev: patch
---

# Jev eval rows split verdict from truth

Jev eval rows split the old pass flag into `verdict_ok`, `truth_ok` and `false_green` and carry per-phase laps, `omitted_actions` and settle timing. The live tier gains `JEV_EVAL_N` repeats, a pinned per-task baseline with mid-run-edit detection, and a search-decoy false-DONE fixture. All of it is test-only code under `fixture_harness`; the one production edit (`Page::settle` returning the page's settle reply) does not change behaviour. Docs updated.
