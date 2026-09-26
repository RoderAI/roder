## 0.1.2 (2026-09-26)

### Fixes

#### Publish today's content under fresh crate versions

`roder-api` 0.1.21, `roder-core` 0.1.19, `roder-app-server` 0.1.17,
`roder-evals` 0.1.2 and `roder-ext-openai-responses` 0.1.10 were published to
crates.io on 2026-09-23 from a branch that predates the Codex agent backend,
the Jev browser tool, the Chrome bridge work and the Grok 4.7 catalog entries.
crates.io versions are immutable, so those numbers cannot carry the current
code and the registry copies do not match the git tags of the same version.

Bump them so the released content reaches crates.io under versions that
describe it. `roder-ext-jev` and `roder-ext-codex-backend` are bumped with
them: both are new crates whose first publish must depend on a registry
`roder-api` that actually contains `roder_api::backend`, which the 0.1.21
registry copy does not.

Adds the package-local READMEs both new crates need to publish.

## 0.1.1 (2026-09-26)

### Features

#### Add Jev Ultrafast as a browser tool provider

Expose a bounded `jev_browse` tool backed by the upstream Jev CDP agent, with
Roder policy approval and a provider-configured `JEV_API_KEY`.

Roder resolves the Chrome DevTools endpoint itself: it reuses one that is
already listening and otherwise starts a visible Chrome on its own profile, so a
browser task no longer fails because nothing debuggable is running. Tasks run in
a foreground tab by default (`foreground: false` for a background tab), and the
typing helper is served from Roder's own harness — the calling turn's model when
it speaks chat-completions, otherwise the first configured chat-completions
provider — instead of requiring an OpenRouter key.

Results now carry `observed_elements` and, when upstream aborts at its action or
model-call budget, `stopped_because` plus the trace collected so far instead of
a bare traceback. A blocked run explains what Jev can target, so a caller can
choose another page rather than substituting a local one. `wait` actions hold
the page for `JEV_WAIT_MS` (default 800ms) rather than upstream's 100ms, and the
text helper's reasoning field follows the provider's accepted shape.

Jev is now a Rust port that runs inside the Roder binary — the agent loop,
action space, decision contract, text helper and a direct CDP client — with
upstream's in-page scripts vendored verbatim. Browser tasks need no Python, no
`uv` and no `browser_harness` daemon. Fixtures recorded from the upstream Python
pin the port's behaviour: the decision request body matches byte for byte, the
observation fingerprint matches hash for hash, and every documented validation
verdict agrees.
