---
name: rust-clippy
description: Use when running or fixing Rust clippy checks in the Gode workspace, including cargo clippy warnings, automatic fixes, and follow-up Rust tests.
---

# Rust Clippy Skill

## Purpose

This skill guides the AI to use `cargo clippy` effectively to maintain code quality, identify idiomatic Rust patterns, and fix issues automatically across the workspace.

## How to use

1. Run `cargo clippy --workspace` to see warnings.
2. If there are automatic fixes available, run `cargo clippy --fix --workspace --allow-dirty --allow-no-vcs` to apply them.
3. Review any manual fixes needed and modify the code using standard editing tools.
4. Ensure no warnings are left and code is clean.
5. You should typically pair this with writing unit and integration tests.
