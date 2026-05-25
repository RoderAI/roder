# Roder Harbor Plan-First Rerun

Date: 2026-05-25

This note records the plan-first Harbor mechanism and the first full rerun of
the 28 tasks that still failed after the xhigh remaining-failures run.

## Mechanism

Plan-first mode is implemented in the Harbor adapter, not as Roder's read-only
policy mode. When `plan_first_enabled` is set in the agent kwargs, the adapter:

1. Runs a planning `roder exec` turn first.
2. Writes planning artifacts to:
   - `roder-plan.md`
   - `roder-plan-events.jsonl`
   - `roder-plan-stderr.txt`
   - `roder-plan-last-message.txt`
3. Extracts the planning thread id from the planning event stream.
4. Rewrites the generated Roder config back to the implementation reasoning.
5. Runs `roder exec resume <thread_id>` for the implementation turn.

The config generator supports:

- `--plan-first`
- `--plan-first-reasoning`
- `--plan-first-soft-timeout-sec`
- `--plan-first-policy-mode`

The v2 full rerun used `medium` reasoning for the planning turn and `xhigh` for
implementation. This avoided making the planning turn spend xhigh budget while
keeping the scored implementation attempt at the requested reasoning level.

## Validation

Smoke validation:

- Job: `roder-tbench-plan-first-smoke-polyglot-rust-c`
- Task: `polyglot-rust-c`
- Result: pass, mean `1.000`, no Harbor errors.

Full v2 rerun:

- Config: `evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh-plan-first-v2.json`
- Job: `evals/harbor/jobs/roder-tbench-remaining-failures-gpt55-xhigh-plan-first-v2`
- Analysis: `evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh-plan-first-v2-analysis.json`
- Comparison: `evals/reports/harbor/remaining-xhigh-vs-plan-first-v2-comparison.md`
- Trials: 28
- Harbor errors: 0
- Passes: 4
- Scored failures: 24
- Mean: `0.14285714285714285`

Converted tasks versus the prior xhigh remaining-failures run:

- `git-leak-recovery`
- `model-extraction-relu-logits`
- `polyglot-rust-c`
- `regex-chess`

There were no regressions within the 28-task rerun set because all 28 were
baseline failures. The comparison report shows missing baseline passes because
the baseline artifact covered 35 tasks and v2 intentionally reran only the 28
remaining failures.

## Plan-First Reliability

Plan-first was exercised across all 28 tasks:

- 28/28 implementation turns recorded the config switch back to `xhigh`.
- 20/28 tasks produced a substantive `roder-plan.md` larger than 100 bytes.
- 6/28 planning turns hit the planning soft timeout before writing a plan:
  - `crack-7z-hash`
  - `make-doom-for-mips`
  - `make-mips-interpreter`
  - `password-recovery`
  - `polyglot-rust-c`
  - `sam-cell-seg`

The initial plan-first run used xhigh for planning and showed that the model
often began solving or prototyping before writing a plan. The v2 mechanism added
`plan_first_reasoning` and tightened the planning prompt, which improved the
number of completed plan artifacts but did not eliminate planning timeouts.

## Takeaways

Plan-first produced four conversions on the previously failing set and is worth
keeping as a targeted rerun mode. It is not yet a universal replacement for the
normal xhigh rerun path:

- It adds meaningful wall time: the 28-task v2 run took about 2h 16m at
  concurrency 4.
- Some planning turns still overrun before writing a plan, especially tasks with
  policy-sensitive or implementation-heavy framing.
- The strongest conversion was the expected cleanup-style task,
  `polyglot-rust-c`.
- Two previous policy-blocked tasks converted: `git-leak-recovery` and
  `model-extraction-relu-logits`.

Recommended next use:

- Keep plan-first as an explicit targeted mode.
- Use it first for tasks where the failure is likely planning, artifact hygiene,
  or policy-framing sensitive.
- Do not run it blindly on every remaining task without considering the extra
  time cost and planning-timeout risk.
