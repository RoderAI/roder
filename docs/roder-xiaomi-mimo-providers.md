# Roder Xiaomi MiMo Providers

Roder exposes Xiaomi MiMo through two first-class provider ids:

- `xiaomi-mimo`: pay-as-you-go Xiaomi MiMo API.
- `xiaomi-mimo-token-plan`: Xiaomi MiMo Token Plan subscription quota.

Both providers use Xiaomi's OpenAI-compatible Chat Completions API at
`/chat/completions`. Roder sends Xiaomi requests with the `api-key` header and
uses `max_completion_tokens`, matching Xiaomi's OpenAI compatibility examples.
Roder does not use Xiaomi's Anthropic-compatible endpoint for these providers.

## Text Models

Use provider/model labels such as:

- `xiaomi-mimo/mimo-v2.5-pro`
- `xiaomi-mimo/mimo-v2-pro`
- `xiaomi-mimo/mimo-v2.5`
- `xiaomi-mimo/mimo-v2-omni`
- `xiaomi-mimo/mimo-v2-flash`
- `xiaomi-mimo-token-plan/mimo-v2.5-pro`

The catalog keeps Xiaomi's model ids exact. It does not alias
`mimo-v2-flash` to `off-v2-flash`.

## Pay-As-You-Go Auth

The pay-as-you-go provider defaults to:

```toml
[providers.xiaomi-mimo]
base_url = "https://api.xiaomimimo.com/v1"
api_key_env = "MIMO_API_KEY"
```

Accepted environment variables:

- `MIMO_API_KEY`
- `XIAOMI_MIMO_API_KEY`
- `RODER_XIAOMI_MIMO_API_KEY`
- `MIMO_BASE_URL`
- `XIAOMI_MIMO_BASE_URL`
- `RODER_XIAOMI_MIMO_BASE_URL`

## Token Plan Auth

The Token Plan provider intentionally requires the exclusive Token Plan base URL
from the Xiaomi subscription page. It does not fall back to the pay-as-you-go
base URL, because Xiaomi treats pay-as-you-go balance and Token Plan quota as
separate billing systems.

```toml
[providers.xiaomi-mimo-token-plan]
base_url = "https://token-plan-cn.xiaomimimo.com/v1"
api_key_env = "MIMO_TOKEN_PLAN_API_KEY"
```

Accepted environment variables:

- `MIMO_TOKEN_PLAN_API_KEY`
- `XIAOMI_MIMO_TOKEN_PLAN_API_KEY`
- `RODER_XIAOMI_MIMO_TOKEN_PLAN_API_KEY`
- `MIMO_TOKEN_PLAN_BASE_URL`
- `XIAOMI_MIMO_TOKEN_PLAN_BASE_URL`
- `RODER_XIAOMI_MIMO_TOKEN_PLAN_BASE_URL`

Token Plan API keys must use Xiaomi's `tp-` prefix. Roder validates the key
prefix and the Token Plan host before sending a request. Localhost base URLs are
allowed for tests.

## Speech Synthesis

Xiaomi TTS is exposed through Roder's speech synthesis surface, not the text
model catalog. The speech synthesis provider ids match the billing provider ids:

- `xiaomi-mimo`
- `xiaomi-mimo-token-plan`

Supported TTS model ids:

- `mimo-v2.5-tts`
- `mimo-v2.5-tts-voiceclone`
- `mimo-v2.5-tts-voicedesign`
- `mimo-v2-tts`

CLI examples:

```sh
roder speech synthesis-providers
roder speech synthesize "Hello from MiMo" \
  --provider xiaomi-mimo \
  --model mimo-v2.5-tts \
  --voice Chloe \
  --audio-format wav \
  --output hello.wav
```

App-server clients can call:

- `speech/synthesis/providers/list`
- `speech/synthesize`

For Xiaomi TTS, Roder places the target spoken text in an assistant-role chat
message and sends the `audio` request object to `/chat/completions`. Voice clone
requests can pass `voiceSample`; Roder converts it to the `data:<mime>;base64`
voice format expected by Xiaomi.

## References

- Xiaomi OpenAI API Compatibility: https://platform.xiaomimimo.com/docs/en-US/api/chat/openai-api
- Xiaomi AI tools overview and Token Plan base URLs: https://platform.xiaomimimo.com/docs/en-US/integration/tools-overview
- Xiaomi model and rate limits: https://platform.xiaomimimo.com/docs/en-US/quick-start/model
- Xiaomi TTS V2.5 guide: https://platform.xiaomimimo.com/docs/en-US/usage-guide/speech-synthesis-v2.5
- Xiaomi TTS V2 guide: https://platform.xiaomimimo.com/docs/en-US/usage-guide/speech-synthesis
