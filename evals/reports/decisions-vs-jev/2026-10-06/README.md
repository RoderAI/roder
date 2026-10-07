# Current Jev backend outperforms the Decisions adapter on browser outcomes

Keep Jev as Roder's default decision backend. In this paired local-fixture
benchmark, Jev passed 63/72 attempts (87.5%) and Decisions passed 47/72 (65.3%).
That is a 22.2 percentage-point outcome gap. There were 18 paired attempts
where only Jev passed, two where only Decisions passed, 45 where both passed,
and seven where both failed.

## Measurements

| Measure | Decisions (`gpt-6-luna`) | Jev (`jev-1.13.0`) |
|---|---:|---:|
| Expected outcome passed | 47/72 (65.3%) | 63/72 (87.5%) |
| Ordinary completion scenarios passed | 35/60 (58.3%) | 51/60 (85.0%) |
| Expected safety/input-stop scenarios passed | 12/12 | 12/12 |
| Median wall time on the same 45 successful pairs | 1.614 s | 1.648 s |
| Median latency of successfully parsed decisions | 182.5 ms | 139.5 ms |
| Decision calls, including failed/refused replies | 225 | 214 |
| Reported input tokens, including failed attempts | 276,940 | 392,389 |
| Refusal responses that stopped a run | 9 | 0 |
| Reported `done` but failed outcome grading | 10 | 3 |

Successful-task wall latency is effectively similar in this sample. The median
within-pair Jev/Decisions wall-time ratio is 0.960, while the ratio of the two
marginal medians is slightly above 1; neither supports a substantial end-to-end
speed advantage. Parsed-decision latency excludes the nine refused Decisions
calls because they produced no decision record. All 144 rows report input
usage. Decisions used 29.4% fewer reported input tokens overall, but this is not
a dollar-cost comparison and includes its early failures. Tokenizers, billing
rates and question encoding differ between providers.

## Where behavior differs

- Jev passed all three repeated-label product, shadow-DOM cart, and framed-form
  attempts; Decisions failed all three in each scenario. The product/cart
  failures include repeated clicks after the requested addition was satisfied.
- Decisions returned nine refusal responses on benign fixtures. The adapter
  stopped without dispatching an action from those responses. This is an
  observed integration failure, not an excluded sample.
- Decisions passed scroll-region registration twice; Jev failed it three times.
- Both failed icon-only selection and Enter-to-search three times. The latter
  illustrates why a model's `done` claim is not the outcome grader.
- Both passed all four irreversible-action scenarios in all three repetitions:
  payment confirmation, deletion confirmation, authorized deletion and reversible
  add-to-cart. Missing-input and dismissed-confirm behavior also passed throughout.

## Method

Code baseline: `16b6f443`, plus the new `decisions_vs_jev` evaluation harness.
Run date: 2026-10-06, America/Los_Angeles. 24 named fixture scenarios, three
repetitions each, two providers: 144 attempts. This is a controlled regression
sample, not an estimate of success on arbitrary public websites.

The harness uses the same Roder browser engine, action space, task goals,
outcome graders, fixture values, browser rules and Chrome instance for both
providers. Every attempt gets a fresh fixture site/page. Providers alternate
order across tasks and repeats; execution is sequential. Both use pooled HTTP
clients, no model-driven text generation and no fallback model. The task timeout
is capped at 60 seconds for both. No failed attempts were rerun or dropped from
this dataset, and no implementation changes were made during the measured run.

This compares the current integrations: Decisions uses the Jev-derived prompts
and adapter, while Jev uses its existing native contract. It is not a comparison
of independently optimized prompts. It also does not compare free-form desktop
control or the separate screenshot/native-computer embedding API. Three repeats
per fixture are correlated; the percentages are descriptive, not confidence
bounds for real-world deployment.

## Reproduce

Configure both API keys using their existing environment or Roder provider
settings. This command makes paid calls to both services:

```sh
JEV_EVAL_TASKS=contact_form,search_autocomplete,select_dropdown,pagination,below_the_fold,twin_by_context,table_row_by_context,delayed_spa,menu_button,styled_checkbox,date_field,icon_by_picture,unlabelled_fields,scroll_region,enter_to_search,shadow_component,frame_form,new_tab,missing_value,confirm_dialog,gate_pay_now,gate_delete_account,gate_authorized_delete,gate_add_to_cart \
JEV_EVAL_REPEATS=3 JEV_REQUIRE_CHROME=1 JEV_MODEL=jev-latest \
  mise exec -- cargo test -p roder-ext-jev --lib decisions_vs_jev -- --ignored --nocapture --test-threads=1
```

The eval's successful process exit means the comparison completed, not that all
outcomes passed. It checkpoints every attempt under Cargo's target directory.

Evidence: [raw attempts](rows.jsonl), [aggregate metrics](summary.json),
[per-task CSV](per-task.csv), and [per-task pass counts](task-table.md).
Regenerate aggregate metrics with `python3 summarize.py rows.jsonl` from this
directory.
