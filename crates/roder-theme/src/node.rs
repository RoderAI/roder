//! Styled-node tree and box model.

use std::borrow::Cow;

#[derive(Debug, Clone)]
pub struct StyledNode<'a> {
    pub id: Option<&'a str>,
    pub classes: Vec<&'a str>,
    pub data: Vec<(&'a str, &'a str)>,
    pub children: Vec<StyledNode<'a>>,
    pub content: NodeContent<'a>,
}

impl<'a> StyledNode<'a> {
    pub fn container() -> Self {
        Self {
            id: None,
            classes: Vec::new(),
            data: Vec::new(),
            children: Vec::new(),
            content: NodeContent::Container,
        }
    }

    pub fn text(s: impl Into<Cow<'a, str>>) -> Self {
        Self {
            id: None,
            classes: Vec::new(),
            data: Vec::new(),
            children: Vec::new(),
            content: NodeContent::Text(s.into()),
        }
    }

    pub fn id(mut self, id: &'a str) -> Self {
        self.id = Some(id);
        self
    }

    pub fn class(mut self, class: &'a str) -> Self {
        self.classes.push(class);
        self
    }

    pub fn data(mut self, name: &'a str, value: &'a str) -> Self {
        self.data.push((name, value));
        self
    }

    pub fn child(mut self, child: StyledNode<'a>) -> Self {
        self.children.push(child);
        self
    }
}

#[derive(Debug, Clone)]
pub enum NodeContent<'a> {
    Text(Cow<'a, str>),
    Container,
    // TODO(rfc): `Widget(WidgetKind)` for delegating to ratatui widgets.
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BoxModel {
    /// CSS order: [top, right, bottom, left]
    pub padding: [u16; 4],
    // TODO(rfc): `margin`, `border` (single|double|rounded|thick).
}
