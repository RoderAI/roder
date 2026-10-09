---
roder-ext-jev: minor
---

# Jev ends runs that go round in circles

Jev ends a run that goes round in circles instead of burning its budget. The fourth identical (page, control) pair, six waits in a row that change nothing, and three stale decisions in a row with nothing new on the page each end the run blocked with the new stop causes `looped` and `unsettled`, and the frontier fallback takes over as it does after a stall. The stale cap compares pages exactly: a page whose only change is digits (a clock, a countdown) counts as progress and is left to the action and model-call budgets, as the same-pair counter already treats it. An unchanged wait no longer hides a stall. New public `JevStopCause::Looped` and `JevStopCause::Unsettled`.
