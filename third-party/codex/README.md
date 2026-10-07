Roder contains adapted Apache-2.0 code from OpenAI Codex commit `8b78f4796605bda8e31329537f5fef036e405e66`.

Derived files: `crates/roder-edit-core/src/patch/{parser,streaming,streaming_tests,streaming_error_tests,seek_sequence,text_file,update}.rs` and `crates/roder-ext-openai-responses/assets/apply_patch.lark`. Files are adapted to Roder filesystem interfaces and error types.

The goal continuation, budget-limit, and objective-update templates under
`crates/roder-core/src/goals/` are adapted from `codex-rs/ext/goal/templates/goals/`
at OpenAI Codex commit `87be737b664`. The accompanying goal lifecycle behavior
uses that implementation as its reference. These templates are covered by the
Apache-2.0 license and OpenAI notice in this directory.
