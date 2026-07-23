# roder-ext-deepseek

First-party Roder inference provider for [DeepSeek Platform](https://api-docs.deepseek.com/),
using DeepSeek's OpenAI-compatible Chat Completions API.

- Provider id: `deepseek`
- Display name: DeepSeek Platform
- Default base URL: `https://api.deepseek.com/v1`
- Auth: `DEEPSEEK_API_KEY` (alias `RODER_DEEPSEEK_API_KEY`) or
  `[providers.deepseek]` in the Roder config.
- Models: `deepseek-chat`, `deepseek-reasoner`, `deepseek-v4-flash`,
  `deepseek-v4-pro`

See `docs/roder-deepseek-provider.md` for setup details.

## Links

- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder
- DeepSeek API docs: https://api-docs.deepseek.com/

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```
