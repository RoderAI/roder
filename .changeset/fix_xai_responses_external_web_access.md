---
roder-ext-openai-responses: patch
---

# Fix xAI/SuperGrok Responses 400 on hosted web search

When using the xAI or SuperGrok provider with hosted web search enabled (cached or live), the Responses mapper was unconditionally emitting `"external_web_access"` on the `web_search` tool object. xAI's backend rejects this key with:

  Argument not supported: external_web_access

Now, for `ResponsesProviderProfile::Xai` (both direct `xai` key and `supergrok` OAuth), we emit a plain `{"type": "web_search"}` tool (the `external_web_access` flag is only sent for OpenAI/OpenRouter profiles that understand it).

The web search tool is still included when the runtime requests hosted web search, so Grok's native search should activate as before.

Updated an Xai profile mapping test to assert the key is omitted.
