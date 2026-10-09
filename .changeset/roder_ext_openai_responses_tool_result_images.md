---
roder-ext-openai-responses: minor
---

# Responses engine reports tool-result image support and replays native computer notes

- Tool-result image support is reported per model through one helper shared with the request mapping. `forwards_tool_result_images` is exported for engines that delegate to the Responses engine.
- Native computer replay now sends a capped, untrusted-labelled notes message beside the screenshot of a successful computer call that has notes. Calls without notes replay unchanged.
