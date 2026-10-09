# Decisions optimization protocol — 2026-10-08

The optimization uses the same 24 tasks as the October 6 paired comparison.
It measures application outcomes, including expected safety/input stops, rather
than accepting the model's DONE claim as success.

1. Screening: one repetition of original adapter, per-question refusal handling,
   native prompts, joint complete-action selection, sequential operation/target
   selection, action-effect evidence, broad completion verification, and Jev.
2. Visual/completion screening: one repetition of action-effect text, screenshots
   plus that text, and a narrower goal-specific completion predicate.
3. Confirmation: three fresh repetitions of original adapter, action-effect text,
   visual Decisions, and Jev, using the normal browser screenshot capability.
4. Held-out validation: three repetitions of the same four providers on four new
   fixture tasks. Fixture contracts were debugged with a deterministic scripted
   client before any model was evaluated. Their prompts were not used for tuning.

Each attempt gets a fresh local site/page. Provider order rotates by task and
repetition; execution is sequential, with pooled HTTP clients and a 60-second
per-task timeout. Values for form filling come from fixture data. Fallback and
generative text completion are disabled for every provider. No attempt is removed
or selectively rerun. Screening and confirmation are distinct datasets; screening
is used for selection and is not pooled with confirmation to claim a success rate.

In the second screening, all providers paid the screenshot-capture overhead,
although only the vision variant received the image. In confirmation and held-out
validation, only the vision variant captures screenshots, through the production
browser capability. Normal results exclude images; eval wire traces retain them
because these are throwaway local fixtures. Raw JSONL files containing images are
compressed with gzip. `analyze.py` accepts either plain or compressed JSONL.

The original variant reproduces the October 6 prompt/response behavior including
aborting the response on any question refusal. It runs against today's endpoint,
so it is a contemporaneous control rather than a reuse of historical scores.
Sequential selection can issue two wire requests per browser decision; wire
request count and summed token usage are recorded separately. Joint selection's
operation/target values are deterministic projections of one validated joint
choice; use the recorded wire distribution for calibration, not those projections.

The completion experiments used a 0.90 predicate cutoff. Confidence diagnostics
sweep candidate cutoffs on model-DONE decisions against final outcome grades.
They are descriptive; repeated fixtures are correlated and this is not a held-out
calibration set for selecting a universal production threshold. Browser-inferred
completion is counted separately because it may end a run after a CLICK decision.

The four held-out fixtures exercise a differently named product, an already
satisfied cart, an Enter-submitted search, and a partially populated preferences
form with a checkbox already in the requested state. During keyless validation,
the search fixture was changed to start empty: this browser only offers Enter for
fields it has typed into, so a pre-populated field was outside the supported action
space. That limitation was not fixed or concealed as a model prompting issue.

Frozen held-out fixture hashes before model evaluation (SHA-256):

- tasks: `8e9ab79d31172f1cd1e1383b67d301d8cb4a14c7c87a746fc1206bb31e81f1a7`
- page: `a59a757a4abddf39e9b1c7764c8c818d9ecada142c33a1d69d4e872f619cff77`

The new fixtures test limited transfer of the design changes. Four task templates
and three repetitions do not establish performance on arbitrary public websites.
Wall times are local end-to-end measurements on a shared development machine and
include browser overhead; they are not isolated API latency or service guarantees.
