---
roder-api: patch
roder-ext-openai-responses: patch
---

# Fix grok-composer-2.5-fast image input handling

The `grok-composer-2.5-fast` model does not support image inputs, but Roder's catalog
hardcoded `supports_images: true` for all xAI/SuperGrok models. The `xai_model` macro now
takes a `supports_images` parameter so non-vision models can correctly declare their
capabilities.

The OpenAI Responses provider engine now checks the model's `supports_images` flag before
emitting `input_image` content items in request payloads. This prevents the xAI API error:

  "Image inputs are not supported by this model."

`grok-composer-2.5-fast` is set to `supports_images: false`; all other Grok models keep
their previous `true` value.
