# Changelog
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
