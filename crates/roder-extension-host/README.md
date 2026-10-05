# roder-extension-host

`roder-extension-host` is the default extension host for [Roder](https://roder.sh).

## What It Does

It assembles the default first-party extension registry, inference providers, web search providers, notifications, and session stores used by the CLI distribution.

## How It Fits Into Roder

Roder is an agentic software development system with a Rust CLI/TUI, a JSON-RPC app-server, SDKs, package resources, and first-party runtime extensions. This package is released as part of that workspace so downstream users can depend on the same component boundaries that Roder itself uses.

## MySQL schema setup

Before starting workers with `SessionStoreConfig::Mysql` or `MysqlOwned`, run
`roder-mysql-migrate` with `RODER_MYSQL_SESSION_URL` supplied through the secret
environment. Schema setup is a release step; constructing a runtime never runs
DDL or waits for a migration lock. Existing schema-version-3 databases need no
data changes. Embedders must use the MySQL store version selected by this host
when constructing configurations and runtime-owner leases.

## Links

- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```
