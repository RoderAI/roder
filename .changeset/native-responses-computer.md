---
roder-api: minor
roder-core: minor
roder-ext-chrome: minor
roder-ext-openai-responses: minor
roder-app-server: patch
---

# Native OpenAI Responses computer use

Register the native `computer` tool for an explicitly bound CDP browser, execute
ordered action batches with retained per-thread sessions and cancellation
cleanup, and return original-detail `computer_screenshot` observations through
`computer_call_output`. Preserve native call identities through transcript
replay, WebSocket continuation, and ACP tool updates. Include an independent
real-browser protocol eval and a live OpenAI eval runner.

Accept the native API's nullable mouse modifiers. Initialize TLS and large
worker stacks in the live runner, preserve call execution order in its trace,
and independently verify the final browser UI and masked screenshot.
