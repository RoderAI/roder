<p align="center">
  <a href="https://roder.sh">
    <img src="https://roder.sh/roder-icon.png" alt="Roder" width="96" height="96">
  </a>
</p>

<h1 align="center">Roder</h1>

<p align="center">A Rust-native coding agent for your terminal, scripts, and editor.</p>

<p align="center">
  <a href="https://roder.sh">Website</a> ·
  <a href="https://github.com/RoderAI/roder#provider-quickstart">Documentation</a> ·
  <a href="https://github.com/RoderAI/roder">GitHub</a>
</p>

<p align="center">
  <a href="https://www.npmjs.com/package/@roderai/roder"><img src="https://img.shields.io/npm/v/@roderai/roder?color=ff4d00" alt="npm version"></a>
  <a href="https://github.com/RoderAI/roder/blob/master/LICENCE"><img src="https://img.shields.io/badge/license-MIT-111316" alt="MIT license"></a>
</p>

Roder reads your code, edits files, and runs commands through a native Rust runtime. Use the interactive terminal interface, run a task from a script, or connect an editor through the Agent Client Protocol (ACP). Choose from providers including OpenAI, Anthropic, Gemini, Codex, and OpenRouter.

## Get started

```sh
npm install -g @roderai/roder
roder --version
roder
```

Or launch without a global installation:

```sh
npx --yes @roderai/roder
```

### Connect a provider

For a ChatGPT subscription, sign in through Roder:

```sh
roder auth login codex
roder auth status
roder
```

Select your provider and model in Roder's provider menu. Other providers support their own API keys, such as `OPENAI_API_KEY` or `ANTHROPIC_API_KEY`. See the [provider quickstart](https://github.com/RoderAI/roder#provider-quickstart) for the full list and configuration examples.

Settings and credentials live under `~/.roder`. Installing or removing this npm package preserves that directory.

## Run from a script

```sh
roder exec "Explain how this repository is organized"
printf 'Review the current diff\n' | roder exec --json -
```

`roder exec` runs without a terminal. Its default stdout contains the final response; `--json` emits JSONL events for automation. See the [execution reference](https://github.com/RoderAI/roder/blob/master/docs/roder-exec.md).

## Connect an editor

Start Roder's ACP endpoint:

```sh
npx --yes @roderai/roder acp
```

For ACP clients that accept a command and arguments:

```json
{
  "command": "npx",
  "args": ["--yes", "@roderai/roder", "acp"]
}
```

With a global installation, use `roder` as the command and `["acp"]` as its arguments. See [Roder's ACP documentation](https://github.com/RoderAI/roder#agent-client-protocol).

## Supported platforms

| Platform | Architecture |
| --- | --- |
| macOS | Apple Silicon / ARM64 |
| Linux | x64 |
| Linux | ARM64 |

Node.js 18 or newer is required to launch the native binary. Linux requires glibc 2.35 or newer and system CA certificates. Alpine/musl, Intel macOS, and native Windows are unsupported; Windows users can run a supported Linux distribution through WSL.

All three native binaries are bundled in the package. No Rust compiler, install scripts, or separate binary download is needed. Installation also works with `npm install --ignore-scripts`.

## Update or remove

```sh
npm install -g @roderai/roder@latest
npm uninstall -g @roderai/roder
```

To pin a release, install `@roderai/roder@<version>`.

Roder is [MIT licensed](https://github.com/RoderAI/roder/blob/master/LICENCE). Report bugs and request features on [GitHub](https://github.com/RoderAI/roder/issues).
