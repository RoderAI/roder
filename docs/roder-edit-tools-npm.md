# Roder Edit Tools NPM Package

`@roderai/edit-tools` is the incremental-adoption package for teams that want Roder's file edit behavior inside their own JavaScript agent loop.

It is intentionally separate from `@roderai/sdk`, which controls Roder app-server sessions, and from any future `@roderai/cli` package that may install or wrap the full binary.

## Migration from hashline-style tools

Read output should be readable source with line numbers for orientation only. Models should copy the exact source span into `old_string` and provide the replacement in `new_string`. Hash prefixes or synthetic line hashes should not be model-facing.

## Profiles

- Use `old-new-string` for Claude-style old/new-string edit and multi-edit flows.
- Use `patch` for GPT/OpenAI-family patch-native flows.
- Use `full` only for explicit experiments; normal requests should advertise one mutating edit surface.

## Release safety

Normal checks use `pnpm pack --dry-run`. Real npm publishing requires explicit approval, package-name confirmation under `@roderai`, npm provenance/token setup, and `RODER_NPM_PUBLISH=1`.
