---
roder-api: minor
roder-ext-openai-responses: minor
roder-tools: patch
---

# Freeform apply_patch on the Responses custom-tool channel

Advertise `apply_patch` on the OpenAI Responses freeform/custom tool channel
(`type:"custom"`) for the gpt-5.5 family, matching the channel the model was
RL-trained to emit patches on. `ToolSpec` gains a `freeform_input_field` marker
(default `None`, so ordinary function tools are unchanged); the Responses
provider serializes marked tools as `type:"custom"`, parses `custom_tool_call`
outputs into the normal tool-dispatch path, and replays their results as
`custom_tool_call_output`. Non-gpt-5.5 models and every other provider keep the
JSON `type:"function"` shape. The `apply_patch` handler accepts both the JSON
`{ "patch": ... }` arguments and the raw freeform body.
