# Roder Image Generation Providers

Roder generates images through a provider-neutral media API. Tool calls
(`media_generate_image`), app-server clients (`media/image/generate`), the SDKs,
and the TUI all use the same canonical request, and every generated image is
persisted as a Roder-owned `MediaArtifact` before it is shown, returned, or
attached to a later turn.

## Providers and models

| Provider id | Models | Display alias | Notes |
|-------------|--------|---------------|-------|
| `openai` | `gpt-image-2` | GPT Image 2 | Primary GPT Image model. |
| `openai` | `gpt-image-1.5`, `gpt-image-1`, `gpt-image-1-mini` | — | Compatibility/legacy ids. |
| `google` | `gemini-3.1-flash-image` | Nano Banana 2 | Supports aspect ratio and `1K`/`2K`/`4K` image sizes. |
| `google` | `gemini-3-pro-image` | Nano Banana Pro | Supports aspect ratio and `1K`/`2K`/`4K` image sizes. |
| `google` | `gemini-2.5-flash-image` | Nano Banana | Supports aspect ratio only (no `imageSize`). |
| `fake` | `fake-image` | Fake Image | Deterministic offline provider; always available. |

Notes on provider semantics:

- OpenAI `gpt-image-*` ids are **direct Image API model ids** used with
  `POST /v1/images/generations` and `POST /v1/images/edits`. The OpenAI
  Responses API hosted `image_generation` tool is a different surface that is
  invoked through a text-capable mainline model (such as `gpt-5.5`); Roder does
  not bridge the hosted Responses tool yet.
- Google Nano Banana display names map to canonical Gemini API model ids as
  listed above. Generation uses `generateContent` with inline image parts, and
  all Gemini image outputs carry a SynthID watermark, which Roder records as
  `generation.watermark: "synthid"` in artifact metadata.
- The media provider id `google` is scoped to media generation; the chat
  inference provider keeps its separate `gemini` id.
- Image generation models live in a dedicated media catalog and never appear in
  chat model pickers or chat model profile lists.

## Configuration

```toml
[media]
artifacts_dir = "~/.roder/artifacts"

[media.image_generation]
default_provider = "openai"
default_model = "gpt-image-2"
max_outputs = 4
max_input_images = 16

[media.image_generation.providers.openai]
enabled = true
api_key_env = "OPENAI_API_KEY"
base_url = "https://api.openai.com/v1"

[media.image_generation.providers.google]
enabled = true
api_key_env = "GEMINI_API_KEY"
default_model = "gemini-3.1-flash-image"
```

- Without configuration, the deterministic offline `fake` provider is the
  default, so image generation works in tests and offline profiles.
- API keys are resolved from `api_key_env` (when set), the standard provider
  keys (`OPENAI_API_KEY`; `GEMINI_API_KEY`, `GEMINI_API_TOKEN`, or
  `GOOGLE_API_KEY`), and the provider menu. Keys are never written to
  transcripts, artifacts, or error messages.
- Base URLs can be overridden per provider via config `base_url` or the
  `RODER_OPENAI_IMAGES_BASE_URL` / `RODER_GOOGLE_IMAGES_BASE_URL` env vars.
- Set `enabled = false` on a provider to keep its extension out of the
  registry entirely.
- Config validation rejects `max_outputs = 0`, empty provider keys, and
  default models that are not in the built-in image catalog for `openai` and
  `google`.

## Tool usage

`media_generate_image` accepts the canonical camelCase request:

```json
{
  "provider": "google",
  "model": "gemini-3-pro-image",
  "prompt": "A polished product hero image for a terminal coding agent",
  "aspectRatio": "16:9",
  "imageSize": "2K"
}
```

```json
{
  "provider": "openai",
  "model": "gpt-image-2",
  "prompt": "Make this screenshot look like a clean launch graphic",
  "action": "edit",
  "inputArtifacts": ["media-image-123"],
  "size": "1536x1024",
  "outputFormat": "png"
}
```

- `inputArtifacts` are Roder media artifact ids; the runtime resolves them into
  inline images before the provider call, so providers never touch artifact
  storage directly.
- `action` is `auto` (default), `generate`, or `edit`. With `auto`, requests
  that carry input images use the provider's edit path.
- OpenAI options: `size` (`auto`, `1024x1024`, `1536x1024`, `1024x1536`),
  `quality`, `outputFormat` (`png`/`jpeg`/`webp`), `background`
  (`transparent` supported), `outputCompression`, `moderation`, `count`, and
  the documented `providerOptions` pass-through key `user`.
- Google options: `aspectRatio` (all Nano Banana models) and `imageSize`
  (`1K`/`2K`/`4K`, Nano Banana 2 and Nano Banana Pro only). Gemini models
  generate one image per request.
- Unsupported option combinations fail with clear errors before any network
  call.

## CLI

```sh
roder media providers [--json]
roder media models [--provider google] [--json]
roder media generate "a tiny blue dot" --provider openai --model gpt-image-2 --size 1024x1024
```

## App-server and SDK

- `media/image/providers/list` lists installed providers, their models, and the
  configured default.
- `media/image/generate` generates images directly and emits
  `media/artifactCreated` / `media/previewReady` notifications per output.
- Existing artifact methods (`media/list`, `media/read`, `media/thumbnail`,
  `media/delete`, `media/attachToTurn`) work unchanged on generated images.

See `docs/app-server/api.md` for request/response examples.

## Storage and attachment

Generated images are written to the media artifact store
(`[media].artifacts_dir`, `RODER_MEDIA_ARTIFACT_DIR`, or `~/.roder/artifacts/`)
with a JSON metadata sidecar including `generation` provenance: provider,
model, revised prompt, watermark scheme, safety summary, and provider response
id. Use `media/attachToTurn` (or the TUI attachment flow) to feed a generated
image back into a later turn as a base64 data URL.

## Live test gates

Normal tests use fake HTTP servers and deterministic bytes; nothing hits
OpenAI or Google by default. The opt-in live smokes are ignored unless
explicitly enabled:

```sh
RODER_OPENAI_IMAGE_LIVE=1 OPENAI_API_KEY=... \
  cargo test -p roder-ext-openai-images --test live_openai_images -- --ignored --nocapture

RODER_GEMINI_IMAGE_LIVE=1 GEMINI_API_KEY=... \
  cargo test -p roder-ext-google-images --test live_google_images -- --ignored --nocapture
```

Both smokes generate a single tiny image into a temp directory and delete it.

## Troubleshooting

- `OpenAI image generation authentication failed (status 401)`: check
  `OPENAI_API_KEY`. Some GPT Image models also require OpenAI organization
  verification; complete verification in the OpenAI dashboard. Do not paste
  API keys into chat transcripts.
- `Gemini image generation authentication failed (status 403)`: check
  `GEMINI_API_KEY` or `GEMINI_API_TOKEN`.
- Quota and rate-limit failures surface the provider status code (`429`) and a
  bounded message excerpt; transient `429`/`5xx` failures of JSON generation
  requests are retried automatically. Multipart edit uploads are never
  retried.
- `Gemini blocked the image generation prompt: <reason>`: the prompt was
  rejected by Gemini safety filters; rephrase the prompt.
- `image provider "..." is not available`: the provider extension is not in
  this distribution or was disabled via `enabled = false`.

## Not implemented in this phase

- Partial-image streaming (requests setting `partialImages` are rejected; only
  final images are ever stored).
- The OpenAI Responses hosted `image_generation` tool bridge.
- Video generation providers.
