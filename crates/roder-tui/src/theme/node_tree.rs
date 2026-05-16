//! Helpers for constructing canonical [`StyledNode`] trees that mirror the
//! TUI surfaces a user can target with a theme. The proof uses this for tests
//! and for any future renderer that wants to route through the cascade
//! directly.

use roder_theme::StyledNode;

/// Cross-cutting tags applied to many nodes (RFC §"Class/ID Registry").
#[derive(Debug, Clone, Copy)]
pub enum CrossCuttingTag {
    Error,
    Warning,
    Muted,
    Accent,
}

impl CrossCuttingTag {
    pub fn class_name(self) -> &'static str {
        match self {
            CrossCuttingTag::Error => "error",
            CrossCuttingTag::Warning => "warning",
            CrossCuttingTag::Muted => "muted",
            CrossCuttingTag::Accent => "accent",
        }
    }
}

/// Build a representative fake transcript timeline as a tree of styled nodes:
///
/// ```text
/// #timeline
///   .timeline-user
///   .timeline-assistant
///     .timeline-thinking
///   .timeline-error.error
///   .timeline-tool[data-status="ok"]
/// ```
pub fn fake_timeline<'a>() -> StyledNode<'a> {
    StyledNode::container()
        .id("timeline")
        .child(
            StyledNode::container()
                .class("timeline-item")
                .class("timeline-user")
                .child(StyledNode::text("what does this repo do?")),
        )
        .child(
            StyledNode::container()
                .class("timeline-item")
                .class("timeline-assistant")
                .child(
                    StyledNode::container()
                        .class("timeline-thinking")
                        .child(StyledNode::text("user wants a summary")),
                )
                .child(StyledNode::text("It is a CLI agent runtime.")),
        )
        .child(
            StyledNode::container()
                .class("timeline-item")
                .class("timeline-error")
                .class("error")
                .child(StyledNode::text("model unreachable")),
        )
        .child(
            StyledNode::container()
                .class("timeline-item")
                .class("timeline-tool")
                .data("data-status", "ok")
                .child(StyledNode::text("read_file: README.md")),
        )
}

/// Sample status-line tree.
pub fn fake_status_line<'a>() -> StyledNode<'a> {
    StyledNode::container()
        .id("status-line")
        .child(
            StyledNode::container()
                .class("segment")
                .class("segment-mode")
                .data("data-id", "mode")
                .data("data-mode", "plan")
                .child(StyledNode::text("mode:plan")),
        )
        .child(
            StyledNode::container()
                .class("segment")
                .class("segment-model")
                .data("data-id", "model")
                .child(StyledNode::text("model:gpt-test")),
        )
}

/// Sample composer tree.
pub fn fake_composer<'a>() -> StyledNode<'a> {
    StyledNode::container()
        .id("composer")
        .child(StyledNode::text(""))
}
