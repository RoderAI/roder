# Roder CLI

[Roder](https://roder.sh) is a native Rust coding agent with an interactive terminal interface and non-interactive execution.

```sh
npm install -g @roderai/roder
roder --version
roder

# Run without a global installation:
npx @roderai/roder acp
```

The package bundles the signed macOS Apple Silicon binary and Linux x64/ARM64 binaries. Linux requires glibc 2.35 or newer; Alpine/musl, Intel macOS, and native Windows are unsupported. Windows users can use a supported Linux distribution through WSL.

Node.js 18 or newer launches the native binary. Installation needs no postinstall scripts, build tools, or subsequent binary download. `npm install --ignore-scripts` works. Configuration and credentials remain in `~/.roder`; installation does not change them.

Update with `npm install -g @roderai/roder@latest`. Pin a version with `npm install -g @roderai/roder@<version>`. Remove it with `npm uninstall -g @roderai/roder`.

See the [provider quickstart](https://github.com/RoderAI/roder#provider-quickstart) for authentication and configuration, and the [execution reference](https://github.com/RoderAI/roder/blob/master/docs/roder-exec.md) for `roder exec`.
