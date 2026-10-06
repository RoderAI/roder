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
