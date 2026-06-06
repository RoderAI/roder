# Roder Inference Routing

Adaptive inference routing lets Roder choose a provider, model, and reasoning
effort for each model call without inserting an external proxy between Roder
and the provider.

The core runtime owns the extension contract, candidate validation, event
emission, and selected-model transparency. Router extensions own policy. The
first bundled router is `local`; it uses local turn signals only and does not
call a cheap classifier model.

## Architecture

- `roder-api::InferenceRouter` is the extension-facing trait.
- `ExtensionRegistry` has an `inference_routers` slot and validates duplicate
  router ids.
- `roder-core` builds a bounded `InferenceRoutingContext` before each model
  request, calls the configured router, validates the selected candidate, and
  emits `inference.routing_decision`.
- Real model transport stays inside normal `InferenceEngine` providers. The
  router never proxies provider traffic.
- The bundled `roder-ext-inference-router` extension registers router id
  `local`.
- Router extensions can advertise selectable Auto options. The app-server
  exposes those options in `providers/list.routingOptions` beside real
  providers/models, not inside any provider's model list.

Manual model selection bypasses routing for normal turns. Auto model selection
stores a typed selection mode with a real baseline provider/model plus router
option identity; the runtime then invokes the selected router before inference.
Explicit per-turn provider or model overrides also bypass routing. If the router
abstains, fails, selects an unavailable candidate, or selects a model that
cannot handle required inputs, the runtime keeps the default selection and
records a fallback/abstain decision when applicable.

## Configuration

Routing is disabled unless `inference_router.enabled` is true and the router id
is configured.

```toml
[inference_router]
enabled = true
router = "local"
profile = "coding"
baseline_provider = "codex"
baseline_model = "gpt-5.5"

[inference_router.extension]
objective = "cost"

[inference_router.extension.tiers.simple]
provider = "codex"
model = "gpt-5.3-codex-spark"
reasoning = "low"

[inference_router.extension.tiers.standard]
provider = "codex"
model = "gpt-5.5-mini"
reasoning = "medium"

[inference_router.extension.tiers.strong]
provider = "codex"
model = "gpt-5.5"
reasoning = "high"

[inference_router.extension.profiles.coding]
default_tier = "standard"
simple_tier = "simple"
standard_tier = "standard"
strong_tier = "strong"
risk_floor_tier = "strong"
classifier_prompt = "Reserved for future classifier comparison."

[inference_router.extension.profiles.coding.risk_floors]
security = "strong"
data_loss = "strong"
infra = "strong"

[inference_router.extension.prices."codex/gpt-5.3-codex-spark"]
input_per_million = 0.10
output_per_million = 0.40

[inference_router.extension.prices."codex/gpt-5.5"]
input_per_million = 2.00
output_per_million = 8.00
```

Profiles can encode different business use cases. For example, an eval profile
can bias toward quality by setting `objective = "quality"` and pointing
`default_tier` at a stronger tier, while an interactive coding profile can use
a cheaper `simple_tier` for small edits and lookups.

Only `enabled`, `router`, `profile`, and `baseline_*` are part of Roder's
generic routing envelope. The bundled local router owns the shape below
`inference_router.extension`; another router extension can define a different
extension table without inheriting local tier or risk-floor vocabulary.

When this config resolves to a registered router and a real baseline
provider/model, the app-server model picker includes an option such as
`Auto: Coding`. If routing is disabled, the router id is missing, or the
baseline is not available, the Auto option is hidden from the main picker and
routing diagnostics remain available through `inference/routing/status` after
turns that attempted routing.

## Local Router Policy

The bundled local router extracts named signals from bounded runtime context:

- phase and runtime profile.
- image input, available tool families, and recent tool family usage.
- prior tool/error failures and prior escalations.
- local keyword-based risk and intent signals from a short latest user-message
  preview.

Risk signals such as `security`, `data_loss`, `infra`, `architecture`, or
`privacy` can force a risk floor tier. Routine intents such as `small_edit`,
`file_lookup`, or `documentation` can use the configured simple tier when no
risk/recovery signal is present.

The router performs compatibility checks before returning a selection:

- provider/model candidate exists in the registered inference engines.
- provider auth is configured where the provider reports auth state.
- image input is supported when the turn has images.
- tool calling is supported when tools are required.
- context window is large enough for the estimated input.
- requested reasoning effort is supported by the candidate model.

## Events and Metrics

Every applied router decision is persisted as `inference.routing_decision`.
`InferenceStarted` also records the actual model and reasoning used for the
provider call, so clients can distinguish default selection from routed
selection.

Use `inference/routing/status` to inspect the latest known routing decision for
a thread or turn, and `inference/routing/metrics` to inspect one turn in more
detail:

```json
{
  "method": "inference/routing/metrics",
  "params": {
    "threadId": "thread-123",
    "turnId": "turn-123",
    "limit": 20
  }
}
```

The response includes raw decision events, outcome counts, selected versus
baseline cost estimates, and regret counters from reliability retries/failures,
turn failure, escalations, and fallbacks.

Cost reporting is an estimate, not billing truth. Version one uses configured
price table values and estimated input tokens at routing time; completion
tokens, cached-token pricing, provider subscriptions, and invoice
reconciliation are not exact. Classifier-overhead fields are reserved for a
future cheap-model classifier comparison.

## Local Lab Kit

The repo includes a small lab kit for trying routing in a TUI session without
touching `~/.roder`:

```sh
scripts/roder-inference-routing-lab-setup.sh
cargo run -p roder-cli -- --config-dir /tmp/roder-routing-lab
```

In the TUI, open the model picker and select `Auto: Coding`. Manual model
choices intentionally bypass routing, so if you leave the picker on a concrete
model the tailer should show provider starts without routing decisions.

In another terminal, follow the latest thread's routing and provider-start
events:

```sh
scripts/roder-inference-routing-lab-tail.sh /tmp/roder-routing-lab --follow
```

After a turn finishes, inspect app-server status and metrics for the latest
thread/turn:

```sh
scripts/roder-inference-routing-lab-metrics.sh /tmp/roder-routing-lab
```

Useful prompts:

- Routine lookup: `Read docs/roder-inference-routing.md and summarize the config keys in 3 bullets.`
- Small edit: `Find one obvious typo in docs/roder-inference-routing.md. If there is one, fix only that typo; otherwise say no change needed.`
- Standard trace: `Trace how a user prompt becomes an AgentInferenceRequest in roder-core and explain where routing is applied.`
- Risk-floor escalation: `Review the privacy and security implications of adaptive inference routing for private company code. Be critical.`
- Recovery signal: `Run a shell command that fails using a clearly nonexistent command, then diagnose what happened and continue.`

The lab defaults to Codex catalog ids. Override them with environment
variables when setting up the lab:

```sh
RODER_ROUTING_PROVIDER=codex \
RODER_ROUTING_SIMPLE_MODEL=gpt-5.3-codex-spark \
RODER_ROUTING_STANDARD_MODEL=gpt-5.4-mini \
RODER_ROUTING_STRONG_MODEL=gpt-5.5 \
scripts/roder-inference-routing-lab-setup.sh /tmp/my-routing-lab
```
