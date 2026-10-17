---
roder-ext-jev: minor
---

# Add semantic DOM evidence and completion audits to visual Decisions

Select `JEV_DECISIONS_PROFILE=dom` for semantic page structure and form/workflow
questions, or `activation` for clipped-field image routing and typed completion
audits. Embedded clients can use `DecisionsBrowserProfile`. Text-only operation
preserves the selected profile. Independent target and operation batches retain
their original answers and aggregate actual usage; existing action gates apply.

Secret fields report observed presence without misleading empty value placeholders.
Component requests and audits reserve the remaining browser decision budget and
count toward reported calls on answered and billed-error paths.
