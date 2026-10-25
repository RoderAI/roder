# Roder npm distribution

The `@roderai/roder` package exposes the `roder` command. It bundles Linux x64, Linux ARM64, and signed macOS ARM64 binaries, following Composal CLI's install-without-scripts approach. It has no runtime npm dependencies or postinstall download.

The source template lives under `crates/roder-cli/npm`, remains private, and uses the CLI crate's version. Knope updates both manifests through the `roder` changeset entry. The packer renders a public package in a separate output directory. The npm CLI distribution has no independent version stream.

## Automatic release

Merging a CLI release preview PR triggers `release.yml`. After all three versioned binaries have been built, signed where applicable, and uploaded, the workflow calls `cli-npm-release.yml` in the same run. This avoids depending on a second workflow being triggered by a tag created with `GITHUB_TOKEN`.

Each build records a manifest containing the CLI version, source commit, target, and archive checksum. Packaging rejects missing targets, mismatched identities, corrupt archives, and symlinked binary entries. It reads only the expected binary member from each archive.

CI installs the resulting tarball with `--ignore-scripts --offline` on all three native platforms. It checks `--version`, `-V`, and `exec --help`, and verifies macOS notarization. Publication uses the same verified tarball, then verifies an installation from the public registry. A matching registry version is an integrity-checked no-op; conflicting content and attempts to move `latest` backwards fail.

The floating GitHub `latest` release does not publish npm versions. Only versioned CLI releases do.

## One-time npm setup

The first package publication and trusted-publisher setup require an npm account with access to the `@roderai` scope.

1. Land and review the packaging change, then prepare a versioned CLI release. Retain the `roder-cli-npm` artifact after the three installation checks pass.
2. If the package does not yet exist, authenticate with `npm login --registry=https://registry.npmjs.org` and publish that reviewed tarball with `npm publish <tarball> --access public --ignore-scripts --registry=https://registry.npmjs.org`. Check the installed public version before proceeding. Do not publish the private source template or an empty placeholder package.
3. In the package's npm settings, add GitHub trusted publishers for organization `RoderAI`, repository `roder`, and workflow `release.yml` for automatic publication. Permit direct `npm publish`. Add a second publisher for `cli-npm-release.yml` if manual recovery is needed. No GitHub environment is configured.
4. Complete the first successful trusted publication within npm's configuration-expiry window. The workflow uses GitHub-hosted runners, Node 22, npm 11.20.0, `id-token: write`, and provenance. It needs no long-lived npm publishing token.

npm validates the calling workflow for reusable workflows, so automatic publication must trust `release.yml`, rather than only the child workflow that contains `npm publish`. See [npm trusted publishing](https://docs.npmjs.com/trusted-publishers/).

With an authenticated current npm CLI, configure those publishers directly:

```sh
npm trust github @roderai/roder --repository RoderAI/roder \
  --file release.yml --allow-publish --yes
npm trust github @roderai/roder --repository RoderAI/roder \
  --file cli-npm-release.yml --allow-publish --yes
npm trust list @roderai/roder
```

An integrity-checked no-op on an already-published version does not exercise OIDC publication. Validate the new publisher with the next versioned release before its expiry window closes.


## Local checks

```sh
mise exec -- python3 scripts/test_release_cli_npm.py
mise exec -- python3 scripts/generate-knope-config.py --check
mise exec -- python3 scripts/check-registry-readmes.py
actionlint .github/workflows/cli-npm-release.yml .github/workflows/release.yml
```

The tests use fake binaries and mocked registry publication. They cover offline installation, argument and stdin transport, environment and working directory, exit codes, signal forwarding, archive validation, deterministic packing, and publication guards. They never publish to npm.

To package real artifacts locally, use a clean checkout of the release commit and the three `roder-*-release` artifact directories merged into one directory:

```sh
mise exec -- python3 scripts/release_cli_npm.py pack \
  --commit "$(git rev-parse HEAD)" \
  --artifacts /tmp/roder-release-artifacts \
  --output /tmp/roder-npm-package
npm install --global --ignore-scripts --offline --no-audit --no-fund \
  --prefix /tmp/roder-npm-install /tmp/roder-npm-package/*.tgz
/tmp/roder-npm-install/bin/roder --version
```

Use a fresh output directory. Python 3.9 or newer and npm are required for local packaging.

## Recovery

Run `cli-npm-release.yml` manually from `master`, supplying the exact `release_sha` and the successful versioned Release run's `artifact_run_id`. The run verifies all three artifact manifests against the checked-out source before packing and native installation checks. It cannot recover a release predating this packaging implementation.

If npm accepted publication but visibility timed out, inspect the registry before retrying. The publish helper polls for up to fifteen minutes and verifies SHA512 tarball integrity. A rerun never overwrites an existing npm version. Source changes require a new versioned release.
