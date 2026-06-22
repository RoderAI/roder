# Synthetic Provider

Roder ships a first-party `synthetic` inference provider backed by
[Synthetic](https://dev.synthetic.new)'s OpenAI-compatible Chat Completions
API. It routes Roder's normal client-executed tool flow, streaming, and usage
through Synthetic with a user-supplied API key.

## Quick start

```toml
provider = "synthetic"
model = "syn:large:text"

[providers.synthetic]
api_key_env = "SYNTHETIC_API_KEY"
base_url = "https://api.synthetic.new/openai/v1"
```

Then select the provider/model with `synthetic/syn:large:text` (or any other
`syn:` alias or concrete `hf:` id).

## Authentication

The API key is resolved, in order, from:

1. Explicit provider config (`[providers.synthetic] api_key`).
2. `SYNTHETIC_API_KEY` (Synthetic's documented variable).
3. `RODER_SYNTHETIC_API_KEY` (Roder alias).
4. `[providers.synthetic] api_key_env` indirection.

Roder never reads `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` for Synthetic, so
credentials cannot be sent to the wrong provider by accident. The provider is
listed even without a key (so the TUI and app-server can show setup state); a
turn started without a key fails locally with setup guidance and makes no HTTP
request.

## Base URL

The default base URL is `https://api.synthetic.new/openai/v1`, matching
Synthetic's endpoint-specific OpenAI reference. Some general getting-started
snippets show `https://api.synthetic.new/v1`; if your account needs that form,
override it via any of:

- `[providers.synthetic] base_url`
- `SYNTHETIC_BASE_URL`
- `SYNTHETIC_OPENAI_BASE_URL`
- `RODER_SYNTHETIC_BASE_URL`

## Models

Synthetic recommends `syn:` aliases over concrete model ids because pinned
model names can 404 when older models rotate out. Roder ships these built-in
aliases (offline-available, no network needed):

| Alias               | Use                  | Image input |
| ------------------- | -------------------- | ----------- |
| `syn:large:text`    | default, large text  | no          |
| `syn:small:text`    | fast, low-cost text  | no          |
| `syn:large:vision`  | large multimodal     | yes         |
| `syn:small:vision`  | small multimodal     | yes         |

Synthetic's "Always-On Models" are included in every subscription and exposed
via the OpenAI-compatible `/models` endpoint. Roder pins them in the catalog so
they are listed offline (no key or network needed) with their documented
context windows:

| Concrete id                                         | Host        | Context     |
| -------------------------------------------------- | ----------- | ----------- |
| `hf:MiniMaxAI/MiniMax-M3`                          | Synthetic   | 512k tokens |
| `hf:Qwen/Qwen3.6-27B`                              | Synthetic   | 256k tokens |
| `hf:moonshotai/Kimi-K2.6`                          | Synthetic   | 256k tokens |
| `hf:nvidia/NVIDIA-Nemotron-3-Super-120B-A12B-NVFP4`| Synthetic   | 256k tokens |
| `hf:zai-org/GLM-4.7`                               | Synthetic   | 198k tokens |
| `hf:zai-org/GLM-4.7-Flash`                         | Synthetic   | 192k tokens |
| `hf:zai-org/GLM-5.1`                               | Synthetic   | 192k tokens |
| `hf:zai-org/GLM-5.2`                               | Synthetic   | 512k tokens |
| `hf:openai/gpt-oss-120b`                           | Fireworks   | 128k tokens |
| `hf:Qwen/Qwen3.5-397B-A17B`                        | Together AI | 256k tokens |

Synthetic still recommends the `syn:` aliases for routing; the concrete ids
are safe to select and stream directly. Other concrete `hf:{owner}/{model}`
ids (for example newly added models) are preserved verbatim across config
parsing, app-server listing, the TUI picker, discovery, and inference. When a
key is configured, Roder discovers the live model list via
`GET <base_url>/models` in the background and merges it into the cache;
provider lists never block on that network call and always fall back to the
built-in aliases and always-on models.

## Live testing

A live smoke test is gated behind environment variables and is ignored by
default:

```sh
RODER_SYNTHETIC_LIVE=1 SYNTHETIC_API_KEY=... SYNTHETIC_LIVE_MODEL=syn:large:text \
  cargo test -p roder-ext-synthetic -- --ignored
```

When `SYNTHETIC_LIVE_MODEL` is omitted it defaults to `syn:large:text`. The
live test lists models and completes one short streaming turn, asserting
non-empty output and streamed usage. Do not record real keys, response bodies,
prompts, quota details, or provider request ids in test output or evidence.

## Deferred surfaces

This phase is API-key Chat Completions inference only. Synthetic's
Anthropic-compatible `/messages` transport, `/embeddings`, `/quotas` status,
and `/search` web search are intentionally deferred to later plans.
