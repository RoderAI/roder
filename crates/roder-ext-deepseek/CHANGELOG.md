# Changelog
## 0.1.2 (2026-08-06)

### Fixes

#### Fix DeepSeek thinking mode reasoning in Ctrl+P and tool rollouts

DeepSeek models advertise real thinking efforts again, stream
`reasoning_content`, send the DeepSeek `thinking` toggle, and pass CoT back on
tool-call turns.

## 0.1.1 (2026-07-23)

### Features

#### Add DeepSeek Platform inference provider

Adds first-class `deepseek` provider support labeled "DeepSeek Platform", using
DeepSeek's OpenAI-compatible Chat Completions API at `https://api.deepseek.com/v1`
with `DEEPSEEK_API_KEY` auth and built-in models `deepseek-chat`,
`deepseek-reasoner`, `deepseek-v4-flash`, and `deepseek-v4-pro`.

## 0.1.0 (2026-07-21)

### Features

#### Add first-party DeepSeek Platform inference provider

Adds the `deepseek` provider ("DeepSeek Platform") using DeepSeek's
OpenAI-compatible Chat Completions API at `https://api.deepseek.com/v1`. The
provider ships built-in models (`deepseek-chat` default, `deepseek-reasoner`,
`deepseek-v4-flash`, `deepseek-v4-pro`), resolves credentials only from
`DEEPSEEK_API_KEY`/`RODER_DEEPSEEK_API_KEY` or `[providers.deepseek]`, and is
visible without credentials so app-server and TUI can show setup state. Turn-time
inference fails locally with setup guidance when the key is missing.
