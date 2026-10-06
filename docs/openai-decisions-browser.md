# OpenAI Decisions browser backend

Roder can use the [OpenAI Decisions API](https://developers.openai.com/api/docs/guides/decisions)
in place of Jev's TypeSafe decision service. Start Roder with:

```sh
export JEV_DECISION_PROVIDER=openai
# Set OPENAI_API_KEY through your normal secret-management workflow.
roder
```

Alternatively, store the key using Roder's existing OpenAI provider configuration
(`[providers.openai] api_key` or `providers/configure` with `provider: "openai"`).
`OPENAI_API_KEY` takes precedence. This endpoint requires an API key; ChatGPT/Codex
sign-in does not authenticate Decisions requests. No Jev key is needed.

The agent continues to call `jev_browse` and `jev_tab_*`. Decisions selects the
operation and observed target using `gpt-6-luna` at `POST /v1/decisions`.
The model is fixed to the model currently supported by that endpoint; `JEV_MODEL`
only configures the Jev backend. Unset `JEV_DECISION_PROVIDER` or set it to `jev`
to use Jev again. Other values fail explicitly.

Both backends share Chrome sessions, action validation, origin restrictions,
timeouts, cancellation, approval policy, fallback tools, and completion checks.
ACP clients use the same tool-call updates and permission flow; this does not
add a protocol method or capability. Browser page text, indexed elements and
recent actions are sent as text evidence. The operation and each potential
operation's target are independent conditional choice questions; only the
selected operation's target is executed. If enabled, the irreversible-action
gate uses predicate questions and fails closed on missing or invalid answers.

Decisions does not generate text to fill fields: the existing Roder text helper
and its credentials remain necessary for typing. The fallback also continues
to use the session's configured model. The trace retains the decision model
and provider-reported usage; omitted token counts stay unknown.

The integration is covered by local HTTP contract tests, including action
selection, predicate mapping, malformed answers and authentication failures.
Those tests do not establish live model quality or account access.

## Native computer choices

Embedding hosts can call `OpenAiDecisionsClient::choose_computer` with a fresh
inline screenshot and `ComputerCandidate` values containing descriptions and
native `ComputerActions` batches. Decisions chooses one supplied batch or
`blocked`; it cannot invent coordinates or tool arguments. The result includes
confidence and provider-reported usage. The host must apply its usual approval,
origin, and stale-observation checks before executing through `run_computer`
or the registered computer tool. This is an embedding API, not a new chat tool
or an automatic replacement for the session's Responses computer model.

## Live validation

With an OpenAI API key configured, run the dedicated evals (these spend API
calls against throwaway local fixture pages):

```sh
JEV_REQUIRE_CHROME=1 JEV_EVAL_TASKS=contact_form,select_dropdown,below_the_fold,gate_pay_now \
  mise exec -- cargo test -p roder-ext-jev --lib decisions_live_browser_corpus -- --ignored --nocapture
JEV_REQUIRE_CHROME=1 mise exec -- cargo test -p roder-ext-jev --lib \
  decisions_live_computer_canvas -- --ignored --nocapture
```

The browser corpus checks resulting DOM/form outcomes with the real Decisions
model. Its field values come from task fixtures to isolate decision quality;
it does not validate text-model generation. It disables fallback so another
model cannot silently complete failed Decisions tasks. Results are written to
`jev-evals/decisions-live-<timestamp>.jsonl` under Cargo's target directory.
The computer test sends an actual screenshot, asks the live model to choose
between two coordinate clicks and `blocked`, executes the native batch, and
checks the canvas success marker. Both tests fail on missing credentials or
Chrome; neither silently skips. An unignored counterpart tests the same native
computer execution with a scripted response, which is not live-model proof.
