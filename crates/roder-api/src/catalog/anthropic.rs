use super::{
    ModelCatalogEntry, OPUS_REASONING, REASONING_HIGH, REASONING_MEDIUM, anthropic_model,
    claude_code_model,
};

pub(super) const OPUS_55: ModelCatalogEntry = anthropic_model(
    "claude-opus-5-5",
    "Claude Opus 5.5",
    "Anthropic's latest Opus model for long-running coding and knowledge work.",
    1_000_000,
    900_000,
    REASONING_MEDIUM,
    OPUS_REASONING,
    true,
);

pub(super) const SONNET_5: ModelCatalogEntry = anthropic_model(
    "claude-sonnet-5",
    "Claude Sonnet 5",
    "Fast, capable Claude model for coding and agent workflows.",
    1_000_000,
    900_000,
    REASONING_HIGH,
    OPUS_REASONING,
    true,
);

pub(super) const CLAUDE_CODE_OPUS_55: ModelCatalogEntry = claude_code_model(
    "claude-opus-5-5",
    "Claude Code Opus 5.5",
    "Claude Opus 5.5 through the local Claude Code harness.",
    1_000_000,
    900_000,
    REASONING_MEDIUM,
    OPUS_REASONING,
);

pub(super) const CLAUDE_CODE_SONNET_5: ModelCatalogEntry = claude_code_model(
    "claude-sonnet-5",
    "Claude Code Sonnet 5",
    "Claude Sonnet 5 through the local Claude Code harness.",
    1_000_000,
    900_000,
    REASONING_HIGH,
    OPUS_REASONING,
);
