# OpenAI service tiers

Set a default tier in your Roder `config.toml`:

```toml
[providers.openai]
service_tier = "ultrafast"
```

Roder sends this as the OpenAI Responses API's `service_tier` field. An explicit
per-request `service_tier_override` takes precedence. Leave the config field
unset to use OpenAI's default behavior.

[OpenAI Ultrafast mode](https://developers.openai.com/api/docs/guides/ultrafast-mode)
is available for supported models including `gpt-6-astra` and `gpt-6.1-sol`,
with separate pricing and rate limits. The config does not change the selected
model. The tier actually served remains recorded in `TokenUsage.service_tier`.

This setting applies to the `openai` API provider. OpenRouter has its own
`[providers.openrouter] service_tier` setting; Codex subscription requests do not
inherit the OpenAI API provider's default.

## API and SDK overrides

Set `serviceTier` on `thread/start` to persist a default for that thread. Update
an existing thread with:

```json
{"method":"thread/set_service_tier","params":{"threadId":"thread-123","serviceTier":"ultrafast"}}
```

Send `serviceTier: null` to that method to restore provider defaults. Set
`serviceTier` on `turn/start` for a single-turn override. Precedence is turn,
thread, then provider config.

TypeScript: `RoderAgent.create({ ..., serviceTier: "ultrafast" })`,
`agent.setServiceTier("ultrafast")`, or `agent.send("...", { serviceTier: "default" })`.
Python: `RoderAgent.create(..., service_tier="ultrafast")`,
`await agent.set_service_tier("ultrafast")`, or
`await agent.send("...", service_tier="default")`.
