---
roder-ext-jev: patch
---

# Multi-step eval tasks, a work gate, and transient provider time kept apart

The eval corpus gains ten multi-step tasks ported from fastbrowse's mock site (a sign-in and basket, an order the shop rejects until its postcode is corrected, a six-item cart, a send that fails with 503 behind a thank-you page, a hidden instruction, a honeypot field, a consent banner with a delayed modal, an infinite feed, a pager to exhaust, and a wizard rejected for a choice two steps back), each graded on what the site holds and checked against runs that stop short and say DONE. The live tally now records median decision calls, text calls and wall time per task, and its baseline comparison flags a task whose median calls, input tokens or decision cost rose past a quarter as `MORE WORK`, which `JEV_EVAL_STRICT=1` fails on; wall time never gates. The hosted decision transport counts the time its requests lose to transient provider failures (retried attempts and backoff waits), and live eval rows report it as `transient_ms`, apart from the wall time.
