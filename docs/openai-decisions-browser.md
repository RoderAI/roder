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
The extension requests credentials for the selected backend only; OpenAI also
declares `network.api.openai.com`. Rotating a configured API key refreshes the
cached browser client on the next call.

The agent continues to call `jev_browse` and `jev_tab_*`. Decisions selects the
operation and observed target using `gpt-6-luna` at `POST /v1/decisions`.
The model is fixed to the model currently supported by that endpoint; `JEV_MODEL`
only configures the Jev backend. Unset `JEV_DECISION_PROVIDER` or set it to `jev`
to use Jev again. Other values fail explicitly.

Both backends share Chrome sessions, action validation, origin restrictions,
timeouts, cancellation, approval policy, fallback tools, and the browser loop.
Decisions uses its own question wording and includes action context and observed
effects. It also requests a fresh viewport screenshot from Chrome on each decision.
Set `JEV_DECISIONS_TEXT_ONLY=1` to use only text and action effects. Embedded hosts
can use `OpenAiDecisionsClient::text_only()` for the same behavior.

Screenshots are suppressed when the observation contains a recognized password or
one-time-code field, or when the run remembers a previously typed secret. The
client then uses the scrubbed text observation. This protects the existing secret
handling boundary; it is not general image redaction or detection of every kind
of sensitive content. A browser implementation without screenshot support also
uses text. Images are decision input and are not included in normal tool results.
ACP clients use the same tool-call updates and permission flow; this does not
add a protocol method or capability. Browser page text, indexed elements,
recent actions and screenshots when available are sent as evidence. The operation and each potential
operation's target are independent conditional choice questions; only the
selected operation's target is executed. Single-target questions are resolved
locally because the API requires at least two choices. If enabled, the irreversible-action
gate uses predicate questions and fails closed on missing, refused or invalid answers.
A refusal on an unused target question does not abort a valid selected action;
a refused operation or selected target stops without dispatching an action.

Decisions does not generate text to fill fields: the existing Roder text helper
and its credentials remain necessary for typing. The fallback also continues
to use the session's configured model. The trace retains the decision model
and provider-reported usage; omitted token counts stay unknown.

The integration is covered by local HTTP contract tests, including action
selection, predicate mapping, malformed answers and authentication failures.
See the [live validation report](validation/decisions-2026-10-06.md) for authenticated browser and native computer results and their limits.

## Configuration reference

These settings are read by the process hosting Roder's built-in `jev_browse`
client. A developer's `.zshrc` does not configure a service, container or remote
Verifier runner. Inject credentials through that service's secret management.

| Setting | Value and behavior |
|---|---|
| `JEV_DECISION_PROVIDER` | `openai` selects Decisions; unset or `jev` selects TypeSafe. Any other value fails. |
| `OPENAI_API_KEY` | OpenAI API credential for Decisions. Takes precedence over `[providers.openai] api_key`. Codex/ChatGPT login is insufficient. |
| `JEV_DECISIONS_TEXT_ONLY` | Unset or `0`: request fresh viewport images when supported and safe. `1`: text only. Other values fail for the OpenAI backend. |
| `JEV_MODEL` | TypeSafe model only; does not change Decisions' fixed `gpt-6-luna` model. |
| `JEV_API_KEY` | Needed only when the built-in TypeSafe backend is selected; switching to Decisions does not require this key. |
| `JEV_CDP_URL` / `JEV_CDP_PORT` | Existing Chrome connection settings; changing the decision provider does not provision a browser. |
| `JEV_TEXT_MODEL_API_KEY` / `OPENROUTER_API_KEY` | Existing text-helper credentials when generated field values are needed. Decisions itself does not generate fill text. A host-supplied resolver can provide values instead. |
| `JEV_FALLBACK` and `JEV_FALLBACK_MODEL` | Existing browser fallback configuration, independent of decision-provider selection. Disable fallback when measuring Decisions outcomes so another model cannot hide failures. |

Example for an already configured Chrome/text-helper environment:

```sh
# OPENAI_API_KEY is supplied by the service secret manager; do not commit it.
export JEV_DECISION_PROVIDER=openai
export JEV_DECISIONS_TEXT_ONLY=0
roder
```

To roll back the built-in client, set `JEV_DECISION_PROVIDER=jev`, ensure its
TypeSafe key is available, and restart the host process. For long-lived embedded
clients, reconstruct the client when configuration changes. Configured OpenAI key
rotation refreshes Roder's cached built-in clients; a custom transport must handle
its own credential lifecycle.

Static distribution metadata lists the union of provider capabilities for
conservative profile preflight. Runtime capability requests are selected-provider
specific: Decisions requests `network.web`, `network.api.openai.com` and
`secret.read.OPENAI_API_KEY`; TypeSafe requests `network.web` and
`secret.read.JEV_API_KEY`. The static list does not require both credentials.

### Embedded clients and Composal Verifier

An embedding host that constructs `JevTypeSafeDecisionClient` explicitly does
**not** use this environment selector. Use `OpenAiDecisionsClient::new(key)` for
direct requests or `OpenAiDecisionsClient::with_transport(transport)` for a
server-owned relay speaking the native Decisions request/response format.
Its `text_only()` method selects text-only operation. Wrapping clients must
forward `uses_images()` and `choose_gated()`, and the browser adapter must implement
`screenshot()` for visual operation. Without screenshot support, it uses text.
Optional screenshot capture errors also fall back to text. Secret-input state,
including short values, suppresses screenshots across built-in session calls.

For Composal Verifier, follow the [migration handoff](verifier-decisions-migration.md).
The current Vex integration explicitly constructs TypeSafe and relays through a
TypeSafe-specific Rails endpoint; environment variables alone cannot migrate it.

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

## Measured optimization

The [October 8 experiment report](../evals/reports/decisions-vs-jev/2026-10-08/README.md)
compares the original adapter, refusal handling, native prompts, complete-action
choices, sequential choices, action-effect history, completion predicates and
screenshots. Experimental variants are internal to the eval harness; they are not
additional production providers. Jev remains the default provider.

The browser host can implement `JevBrowser::screenshot()` to return an inline
viewport image; `None` means it has no image capability. Decision wrappers should
forward `JevDecisionClient::uses_images()`. Existing ACP tool names, approval
requests and tool-call updates are unchanged; no new protocol capability is advertised.

The [follow-up Jev hill-climb](../evals/reports/jev-hillclimb/2026-10-08/README.md)
measures improvements to the default TypeSafe client against its original profile
and this visual Decisions integration on a larger, more demanding corpus. Jev's
form and workflow guidance is isolated from the Decisions prompt profile. Both
continue through the same ACP tool and permission flow.
