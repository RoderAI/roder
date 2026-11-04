# roder-ext-hopper

First-party Roder inference provider for [Hopper](https://withhopper.com),
fast inference for open-source models behind an OpenAI-compatible Chat
Completions API.

- Provider id: `hopper`
- Display name: Hopper
- Default base URL: `https://api.withhopper.com/v1`
- Auth: `HOPPER_API_KEY` (alias `RODER_HOPPER_API_KEY`) or
  `[providers.hopper]` in the Roder config.
- Models: `deepseek-v4.1-flash-1017` (default), `deepseek-v4.1-flash-1017-abliterated`

See `docs/roder-hopper-provider.md` for setup details.

## Links

- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder
- Hopper: https://withhopper.com
- API keys: https://withhopper.com/console/keys

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```
