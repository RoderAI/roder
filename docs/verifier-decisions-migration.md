# Composal Verifier: Decisions migration handoff

Work in `/Users/pz/w/vex-git`. Read its AGENTS instructions and preserve unrelated
work. Upgrade to the released Roder revision containing RoderAI/roder PR #109,
then switch scenario browser action selection from TypeSafe to OpenAI Decisions.
Resolve and record the immutable release tag/SHA and exact package versions;
do not use a floating master dependency or assume a CLI upgrade updates the
embedded Rust crates. Roder packages have independent versions.

## Current integration, inspected 2026-10-08

- `vex-roder/Cargo.toml` is an excluded standalone workspace with its own lockfile;
  it currently consumes registry `roder-ext-jev = "0.4.1"` and several independently
  pinned Roder crates. Inspect the published dependency closure before upgrading.
- `vex-roder/src/verify/jev.rs` constructs a lease-authenticated `RelayTransport`,
  reads `VEX_VERIFY_JEV_MODEL`, and caps serialized requests at 128 KiB.
- `vex-roder/src/verify/jev/diagnostics.rs` explicitly constructs
  `JevTypeSafeDecisionClient` and adds browser diagnostics under TypeSafe's `state`.
- `vex-api/app/controllers/api/v1/agent_swarm_leases_controller.rb` implements
  `evaluate_verify_jev`, accepts only an active scenario lease, caps the body at
  150,000 bytes, and calls `Typesafe::JevClient` with `verify_jev` billing context.
- `SupervisorBrowser` currently has no screenshot implementation. Merely selecting
  a visual client would therefore run text-only.

## Implementation

1. Upgrade the compatible Roder crate closure and `vex-roder/Cargo.lock`. Confirm
   the registry package includes PR #109. If registry publication is unavailable,
   report it; an explicit immutable Git revision can be used coherently across the
   Roder dependency closure, never a mixture producing duplicate API types.
2. Select `OpenAiDecisionsClient::with_transport` in the scenario browser factory.
   Keep the credential in the trusted API service, not in the runner. The relay
   must send native Decisions JSON to `https://api.openai.com/v1/decisions` and
   return native answers/refusals/usage. Preserve lease ownership, organization,
   revocation, cancellation, rate limits and timeout checks. Do not expose a
   general arbitrary-URL/model proxy. Use the supported `gpt-6-luna` model.
3. Update diagnostics injection for native Decisions input and question schemas.
   Preserve per-request isolation and mark browser diagnostics as untrusted
   evidence. The existing `/state` mutation cannot be reused verbatim.
4. Forward `uses_images()` and gated selection through decision-client wrappers.
   Implement current-viewport screenshot capture in `SupervisorBrowser` using
   the same authorized browser session; preserve freshness/origin checks and
   suppress screenshots around recognized or previously supplied secrets,
   including opaque value references across separate scenario calls. Never log
   image payloads, raw secrets or credentials. Respect screenshot suppression
   independently of text-redaction length limits.
5. Bound image dimensions/encoding and increase both relay body limits together
   only as needed for the selected image bound. Test oversized payload rejection.
   Current 128 KiB/150,000-byte limits are unsuitable as an unexamined assumption
   for base64 screenshots. A text-only rollout is an explicit measured fallback,
   not evidence that visual Decisions was enabled.
6. Keep the existing host value resolver and assigned-action receipts. Decisions
   chooses actions; it does not generate field values. Retain browser permissions,
   cookie refusal and secret-resolution boundaries. Label usage and pricing for
   the actual OpenAI provider/model, and keep absent token counts unknown.
7. Configure the API service's environment-scoped OpenAI credential through Com
   secrets. Verify the selected org/project/environment. Do not copy a local
   `.zshrc` credential into a repo, prompt, runner image, log or report. Add an
   explicit rollout/rollback setting at the scenario factory and relay; the
   built-in `JEV_DECISION_PROVIDER` switch alone does not control this embedding.
8. Keep TypeSafe use in unrelated PR classification or merge resolution unchanged.
   Decide whether to rename provider-specific scenario mode/route names to their
   canonical forward API, updating all callers and generated routes together;
   do not add compatibility aliases just to preserve historical Jev naming.

## Acceptance and rollout

- Add Rust wrapper/adapter and Rails relay tests for native requests, selected and
  unused refusals, safety predicates, usage, authentication, foreign/revoked leases,
  provider failure, oversized images and secret-safe screenshots across calls.
- Test a canary scenario with fallback disabled and prove its decision trace used
  OpenAI Decisions, the released Roder revision, and actual image input where
  allowed. Check final browser state, exact-once mutations and assigned-action
  receipts; a provider label or model DONE answer alone is not proof.
- Exercise a typed form, multi-step review/commit, scoped record selection and a
  secret-input flow. Compare current TypeSafe baseline versus Decisions using
  identical scenarios, recording success, latency and provider-reported usage.
- Build and pin the runner/service artifact that actually embeds the upgraded
  crates. Respect existing project deployment holds and release rules. Report
  merged code, published artifact, deployed service and hosted canary proof
  separately. Restart/recreate cached embedded clients after switching.
- Roll back the scenario provider and deployed artifact coherently if the canary
  regresses. Preserve evidence and operation IDs; do not silently fall back while
  reporting the Decisions rollout as successful.

Deliver a PR, the exact release/crate/artifact pins, secret setting names (never
values), tests, canary evidence and rollback instructions. Read
[configuration](openai-decisions-browser.md) and the
[Jev/Decisions paired study](../evals/reports/jev-hillclimb/2026-10-08/README.md).
The local fixture study is not hosted Composal Verifier proof.
