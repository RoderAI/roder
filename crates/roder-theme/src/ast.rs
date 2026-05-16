//! AST for the supported CSS subset.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    /// Variables collected from any `:root { --name: value; }` blocks.
    pub variables: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// One or more comma-separated selectors.
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    /// 1-based source order, used as a cascade tiebreaker.
    pub source_order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// Compound selectors joined by combinators, leftmost first.
    /// `a b > c` -> [(a, Descendant), (b, Child), (c, None)] where the last
    /// combinator is unused (it joins to "nothing on the right").
    pub parts: Vec<(SimpleSelector, Combinator)>,
}

impl Selector {
    /// Standard CSS specificity tuple: `(ids, classes+attrs+pseudos, types)`.
    pub fn specificity(&self) -> (u32, u32, u32) {
        let mut a = 0;
        let mut b = 0;
        for (simple, _) in &self.parts {
            if simple.id.is_some() {
                a += 1;
            }
            b += simple.classes.len() as u32;
            b += simple.attrs.len() as u32;
            b += simple.pseudos.len() as u32;
        }
        (a, b, 0)
    }

    /// The rightmost simple selector — what matching keys off of for a node.
    pub fn key(&self) -> &SimpleSelector {
        &self
            .parts
            .last()
            .expect("selectors are never empty after parsing")
            .0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SimpleSelector {
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSelector>,
    pub pseudos: Vec<String>,
}

impl SimpleSelector {
    pub fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.classes.is_empty()
            && self.attrs.is_empty()
            && self.pseudos.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrSelector {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// `a b`
    Descendant,
    /// `a > b`
    Child,
    /// End of the chain (rightmost simple selector).
    None,
    // TODO(rfc): adjacent sibling (`+`) and general sibling (`~`).
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    pub value: Value,
    pub important: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Raw token sequence — interpreted by `properties` per declaration name.
    /// Storing as a string keeps the AST small and lets variable substitution
    /// happen at cascade time.
    Raw(String),
}
