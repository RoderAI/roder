# roder-ext-synthetic

First-party Roder inference provider for [Synthetic](https://dev.synthetic.new),
using Synthetic's OpenAI-compatible Chat Completions API.

- Provider id: `synthetic`
- Default base URL: `https://api.synthetic.new/openai/v1`
- Auth: `SYNTHETIC_API_KEY` (alias `RODER_SYNTHETIC_API_KEY`) or
  `[providers.synthetic]` in the Roder config.
- Models: recommended `syn:` aliases (`syn:large:text` default), the 10
  documented "Always-On" concrete `hf:` models pinned offline, plus any other
  `hf:{owner}/{model}` ids discovered from `GET <base_url>/models`.

See `docs/roder-synthetic-provider.md` for setup and live-test details.
