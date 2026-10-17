# Jev completion activation audit

The frozen candidate passed **165/166 attempts with zero false successes**, versus
162/166 with three false successes for the preceding clipped-image Jev profile,
and 160/166 with five false successes for Visual Decisions. These are repeated
synthetic browser fixtures, not a production success-rate estimate.

| Suite | Visual Decisions | Clipped Jev | Audited Jev |
|---|---:|---:|---:|
| Basic regression | 69/72 | 71/72 | 72/72 |
| Compound development | 14/16 | 16/16 | 16/16 |
| Compound regression | 18/18 | 18/18 | 18/18 |
| Search regression | 12/12 | 12/12 | 12/12 |
| Earlier transfer, now regression | 23/24 | 21/24 | 23/24 |
| Completion development | 12/12 | 12/12 | 12/12 |
| New completion transfer | 12/12 | 12/12 | 12/12 |
| **Total** | **160/166** | **162/166** | **165/166** |
| False successes | 5 | 3 | 0 |

The audit recovered **Paris selection followed by Go in 3/3 trials**, versus
0/3 for clipped Jev. All three proposed corrections in the entire campaign were
this missing Go click, and the final page independently confirmed it. See
[the action trace and raw Nouls](escape-trace.json), the original viewport
[before Go](pending-go.jpg) and [after Go](completed-go.jpg). These JPEGs are
exactly decoded model input assets, with no pixel edits.

Basic regression had 39 audits and **no proposed corrections**. Its 72-vs-71
result is not evidence the audit fixed that task. The date-field improvement and
login regression occurred without an audit correction and remain run variation.
The candidate's one failure, `login_form#1`, submitted the expected credentials
but chose BLOCKED on the generic “Thanks, we received it.” receipt. Its original
head assigned DONE and BLOCKED 0.49 each, with confidence 0.23. No audit ran on
that attempt. This ambiguity is retained rather than converted to a success.

All 12 attempts per provider on the four new completion-transfer tasks passed,
including downloading exactly one copy and deliberately repeating an addition
before locking the cart. The candidate did not duplicate the already-queued
receipt or execute the forbidden Apply action in completion development.

| Whole-campaign resource | Visual Decisions | Clipped Jev | Audited Jev |
|---|---:|---:|---:|
| Provider request records | 593 | 586 | 700 |
| Decision input tokens | 1,134,231 | 3,631,732 | 3,818,731 |
| Median task wall time | 2,396.5 ms | 1,518.5 ms | 1,577.5 ms |

The candidate made 110 text-only audits. Relative to clipped Jev, total recorded
requests rose 19.5% and decision input tokens 5.1%. The other four requests include
additional useful actions and run variation. Among the 161 paired attempts both
Jev profiles passed, the median candidate wall-time increase was 63 ms. Latencies
include local browser work and some concurrent validation; they are not an
isolated production benchmark. Model responses identify `jev-1.14.0` and
`gpt-6-luna`; no recorded transport errors occurred. Both Jev profiles scoped
images in 13 requests and otherwise used shared images or secret-safe text.

The seven datasets contain all 498 scheduled attempts on 58 distinct tasks.
Expected values and oracle probes stay outside model requests. Fill values come
from fixture resolvers; generative completion and fallback are disabled. Each
attempt checks both the terminal contract and independent page/side-effect
oracles. The earlier [clipped-image campaign](../target-images/README.md) is
preserved separately; its former holdouts are regression data here.

Candidate source and the campaign protocol are frozen before the new completion
transfer tasks receive paid model calls. `freeze.json` records the exact source
commit, source/fixture hashes and isolated test-binary hash. `protocol.json`
schedules 498 attempts, 166 per provider. Prior tasks are known regression data;
only the four completion-transfer tasks are new to paid model evaluation here.

The candidate extends `dom_targets` with three independent text Nouls when the
original operation is DONE and its conditional click target is usable. The
estimated probabilities concern whether that activation is required, performed,
and ready. Code computes `max(0, required + ready - performed - 1)` and resolves
the original offered CLICK only above 0.5. This is conservative support applied
to model estimates, not a calibrated correctness guarantee or raw Choice
confidence. Conditional target probabilities and irreversible-action gates
remain intact. All extra billed requests count toward usage. This applies
TypeSafe guidance to use [small Noul judgments](https://docs.typesafe.ai/primitives/noul.md)
and [batch independent questions](https://docs.typesafe.ai/patterns/fan-out.md),
while retaining [Choice distributions](https://docs.typesafe.ai/primitives/choice.md).

Development evidence is separate from the frozen campaign:

- `scripted-states.json.gz`: 23 actual browser states from eight scripted tasks;
  expected labels are metadata outside the model request.
- `probe-*.jsonl.gz`: 640 paid recorded-state experiments, including failures.
  Broad completion wording, identifier alignment, image removal, joint choices,
  activation choices and another full-choice retry did not reliably solve the
  known premature DONE. The three-Noul probe motivated the implementation.
- `probe-replan.jsonl.gz`: four of 14 exploratory requests reused an equivalent
  fixture state at another browser port for their preceding assessment. Records
  identify the assessment source URL and mismatch. These are not matched-state
  validation. Actual live audits use the current request state.
- `pilot-escape.jsonl.gz`: nine attempts; candidate 3/3, old clipped profile 0/3,
  Visual Decisions 3/3. Candidate made six wire requests per attempt, including
  two audits, versus three for the failing old profile.
- `pilot-completion-dev.jsonl.gz`: 36 attempts; all three variants passed 12/12,
  including already-complete and explicitly forbidden-submission cases. Each
  candidate attempt made one additional audit.

Reproduce with an isolated library test executable built at `freeze.json`'s
`code_commit`, using Rust 1.95.0, Chrome, and configured OpenAI/TypeSafe keys:

```sh
python3 verify_source.py
JEV_EVAL_BINARY=/absolute/path/to/copied-test-binary python3 reproduce.py
python3 summarize.py
python3 summarize_probes.py
python3 inspect.py confirmation.jsonl.gz complex-dev.jsonl.gz compound-regression.jsonl.gz search-regression.jsonl.gz transfer.jsonl.gz completion-regression.jsonl.gz completion-transfer.jsonl.gz
```

The reproduction runner verifies the executable, protocol and current source bytes, sets the
required stack size, and refuses to overwrite data or logs. Use `--output` with
a new output directory. An independently rebuilt executable must have its own
verified freeze if its digest differs. `task-grid.json` was derived from the
original protocol and fixtures at the frozen source commit; summary and pooled
validators reject substituted task IDs as well as duplicate or missing attempts.
This later helper repair leaves the original protocol and measured data intact.
`audits.py` separately lists every audit and proposed correction, without treating
a proposal as proof that the browser executed it. Model calls are paid. Raw requests and responses, not expected oracle labels,
are sent to providers. Metadata and oracle probes remain outside those requests.

Validation: the complete library suite passed 732/732 with four test threads,
Chrome required, and the repository's `RUST_MIN_STACK=16777216` setting. All 85
integration tests and one doc test passed. Fourteen paid/live tests are ignored
by the ordinary suite. Package Clippy with `--no-deps -D warnings`, formatting
and release gates passed. Dependency-inclusive Clippy reports three existing
`roder-api` lints. Earlier timing failures, successful isolated reruns, and a
direct-binary invocation that omitted the stack setting are retained alongside
the successful full run in `validation.json` and compressed logs.
