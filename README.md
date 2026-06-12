# demo-roder-package

A complete example Roder package. The `roder.toml` at the repository root is
the install contract: it names the package and declares the bundled
resources.

| Resource | Path | What you get |
| --- | --- | --- |
| Skill | `skills/repo-tour/` | `repo-tour` skill for repository orientation |
| Command | `commands/standup.md` | `/standup [days]` slash command |
| Theme | `themes/neon-dusk.css` | `neon-dusk` TUI theme |
| Extension | `extensions/wordtools/` | Python process extension with a `word_count` tool |

## Try it

```sh
# From a checkout of this repository:
roder install ./examples/packages/demo-roder-package

# Or install straight from git (any repo with a root roder.toml works):
roder install git:github.com/<you>/<repo>

roder packages list
roder packages resources demo-roder-package
roder packages approve demo-roder-package   # allow the Python extension to launch
```

Skills, commands, and themes activate immediately. The process extension
stays inert until you approve it. See `docs/roder-packages.md` for the full
contract.
