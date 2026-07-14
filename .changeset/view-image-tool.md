---
roder-tools: minor
roder-api: minor
roder-ext-openai-responses: minor
---

# Path-based `view_image` tool for vision tasks

Adds a native `view_image(path)` tool that mirrors Codex's semantics: it reads
an image file (png/jpeg/gif/webp, validated by magic bytes, capped at 10 MiB),
base64-encodes it, and returns it as an image content block in the tool result
so the model sees the pixels. It reads through the workspace backend, so it
works against both local and remote-runner workspaces.

- `roder-tools`: new `view_image` tool (registered alongside the builtin coding
  tools); a `read_bytes` method on the workspace backend for binary reads; and
  `media_attach` now degrades to actionable guidance (pointing at `view_image`)
  instead of hard-failing when called without raw base64 bytes, so it no longer
  burns the consecutive-tool-failure budget in headless/eval runs.
- `roder-api`: `VIEW_IMAGE_DISPLAY_KEY`, a reserved `display_payload` key that
  carries the image block from tool result to provider.
- `roder-ext-openai-responses`: `function_call_output` now forwards a
  `view_image` result as an `input_image` content block (when the model
  supports images), falling back to the plain string output otherwise.
