---
name: repo-tour
description: Use when a user asks for an orientation tour of an unfamiliar repository. Produces a structured tour of entry points, key modules, and how to run tests.
---

# Repo Tour

Give newcomers a fast orientation of the current repository.

## Steps

1. Identify the build system and language from manifest files (`Cargo.toml`, `package.json`, `pyproject.toml`, `Makefile`).
2. List the top-level directories with a one-line purpose each.
3. Find the main entry points (binaries, servers, CLIs) and name the files.
4. Locate the test suites and state the exact commands to run them.
5. End with three suggested first files to read, in order.

Keep the tour under 30 lines. Prefer file paths over prose.
