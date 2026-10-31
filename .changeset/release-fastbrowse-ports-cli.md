---
roder: patch
roder-extension-host: patch
---

# Ship the fastbrowse browser improvements in the CLI

Include the Jev browser changes ported from fastbrowse in the distributed Roder
CLI: request-aware settle, faster installed snapshots, leaner decision
requests, cross-origin frame sessions with a deadline, downloads, self-clearing
interstitials, one text call per form, and a wider irreversible-action gate.

On the shared live corpus, false greens fell from 1 to 0 and decision cost fell
about 11%. MiniWoB++ successes rose from 83/96 to 87/96. The consult and light
recovery tiers stay off by default (`JEV_CONSULT_BELOW`, `JEV_LIGHT_RECOVERIES`).
