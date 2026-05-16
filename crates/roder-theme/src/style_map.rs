//! A compiled stylesheet ready to resolve styles for [`StyledNode`] queries.

use ratatui::style::Style;

use crate::ast::Stylesheet;
use crate::cascade::{ComputedStyle, compute};
use crate::node::{BoxModel, StyledNode};

#[derive(Debug, Clone, Default)]
pub struct StyleMap {
    sheet: Stylesheet,
}

impl StyleMap {
    pub fn new(sheet: Stylesheet) -> Self {
        Self { sheet }
    }

    pub fn from_css(input: &str) -> Result<Self, crate::parser::ParseError> {
        Ok(Self::new(crate::parser::parse(input)?))
    }

    pub fn sheet(&self) -> &Stylesheet {
        &self.sheet
    }

    /// Resolve a node with full ancestor chain (root-first). For an isolated
    /// node use [`StyleMap::resolve`].
    pub fn resolve_chain<'a>(&self, chain: &[&StyledNode<'a>]) -> (Style, BoxModel) {
        let computed = self.computed_chain(chain);
        (computed.to_ratatui(), computed.box_model())
    }

    pub fn computed_chain<'a>(&self, chain: &[&StyledNode<'a>]) -> ComputedStyle {
        compute(&self.sheet, chain)
    }

    pub fn resolve<'a>(&self, node: &StyledNode<'a>) -> (Style, BoxModel) {
        self.resolve_chain(&[node])
    }

    pub fn computed<'a>(&self, node: &StyledNode<'a>) -> ComputedStyle {
        self.computed_chain(&[node])
    }
}
