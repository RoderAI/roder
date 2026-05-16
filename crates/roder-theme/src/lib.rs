//! `roder-theme` — a small CSS subset engine for terminal UI theming.
//!
//! This is the proof-of-concept implementation for RFC 0001. It implements:
//!
//! * a hand-rolled parser for the subset (id / class / attribute / descendant
//!   combinator / comma groups, `:root` variables, `var(--name)`).
//! * cascade + specificity resolution producing a [`ComputedStyle`].
//! * a [`StyledNode`] tree and [`StyleMap`] that wraps a [`Stylesheet`] and
//!   answers `(ratatui::Style, BoxModel)` queries.
//!
//! See `rfc/0001-tui-css-theming.md` for the full design. TODOs in code mark
//! places where the RFC asks for more than this proof builds.

pub mod ast;
pub mod cascade;
pub mod node;
pub mod parser;
pub mod properties;
pub mod style_map;

pub use ast::{Combinator, Declaration, Rule, Selector, SimpleSelector, Stylesheet, Value};
pub use cascade::{ComputedStyle, FontStyle, FontWeight, TextDecoration};
pub use node::{BoxModel, NodeContent, StyledNode};
pub use parser::{ParseError, parse};
pub use properties::{BorderShape, Display};
// Re-export the property parser helpers — callers (e.g. roder-tui) reuse
// `parse_color` to interpret `:root` variable values directly.
pub use style_map::StyleMap;
