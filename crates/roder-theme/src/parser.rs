//! Hand-rolled parser for the supported CSS subset.
//!
//! Grammar (informal):
//!
//! ```text
//! stylesheet := rule*
//! rule       := selectors '{' declarations '}'
//! selectors  := selector (',' selector)*
//! selector   := simple ((' '|'>') simple)*
//! simple     := (id | class | attr | pseudo)+
//! id         := '#' ident
//! class      := '.' ident
//! attr       := '[' ident ('=' value)? ']'
//! pseudo     := ':' ident                       // also ':root' magic for vars
//! declaration:= ident ':' value ('!important')? ';'?
//! ```
//!
//! Unsupported productions don't hard-fail; we skip to the next `}` to remain
//! resilient. A `ParseError` only fires for truly malformed input.

use crate::ast::*;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unexpected end of input at line {line}")]
    UnexpectedEof { line: usize },
    #[error("unexpected character {ch:?} at line {line}:{col}")]
    Unexpected { ch: char, line: usize, col: usize },
    #[error("stylesheet exceeded size limit: {size} > {limit}")]
    TooLarge { size: usize, limit: usize },
}

/// Hard upper bound, matches RFC §"Security & Safety".
pub const SIZE_LIMIT_BYTES: usize = 256 * 1024;

pub fn parse(input: &str) -> Result<Stylesheet, ParseError> {
    if input.len() > SIZE_LIMIT_BYTES {
        return Err(ParseError::TooLarge {
            size: input.len(),
            limit: SIZE_LIMIT_BYTES,
        });
    }
    let mut p = Parser::new(input);
    let mut sheet = Stylesheet::default();
    let mut order = 0usize;
    p.skip_ws_and_comments();
    while !p.is_eof() {
        match p.parse_rule(order) {
            Ok(Some(rule)) => {
                // :root variable block — extract var declarations and drop.
                // Also expose `background` / `background-color` declarations
                // as the synthetic `background` variable so themes can write
                // either `:root { background: ... }` or `:root { --background: ... }`.
                if rule.selectors.iter().all(|s| s.is_root_pseudo()) {
                    for decl in &rule.declarations {
                        let Value::Raw(v) = &decl.value;
                        if let Some(name) = decl.name.strip_prefix("--") {
                            sheet
                                .variables
                                .push((name.to_string(), v.trim().to_string()));
                        } else if decl.name == "background" || decl.name == "background-color" {
                            sheet
                                .variables
                                .push(("background".to_string(), v.trim().to_string()));
                        } else if matches!(
                            decl.name.as_str(),
                            "border-style" | "border-radius" | "border-color" | "border"
                        ) {
                            // Mirror the declaration as a synthetic variable
                            // so the TUI side can resolve border state from
                            // `:root` without traversing the rule set.
                            sheet
                                .variables
                                .push((decl.name.clone(), v.trim().to_string()));
                        }
                    }
                } else {
                    sheet.rules.push(rule);
                    order += 1;
                }
            }
            Ok(None) => {} // recovered from a bad rule
            Err(e) => return Err(e),
        }
        p.skip_ws_and_comments();
    }
    Ok(sheet)
}

impl Selector {
    fn is_root_pseudo(&self) -> bool {
        self.parts.len() == 1 && {
            let (s, _) = &self.parts[0];
            s.id.is_none()
                && s.classes.is_empty()
                && s.attrs.is_empty()
                && s.pseudos == ["root"]
        }
    }
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            src: input.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => {
                    self.bump();
                }
                Some(b'/') if self.src.get(self.pos + 1) == Some(&b'*') => {
                    self.bump();
                    self.bump();
                    while !self.is_eof() {
                        if self.peek() == Some(b'*') && self.src.get(self.pos + 1) == Some(&b'/') {
                            self.bump();
                            self.bump();
                            break;
                        }
                        self.bump();
                    }
                }
                _ => return,
            }
        }
    }

    fn parse_rule(&mut self, order: usize) -> Result<Option<Rule>, ParseError> {
        // Capture selector chunk up to '{'
        let mut sel_buf = String::new();
        let start_line = self.line;
        while let Some(c) = self.peek() {
            if c == b'{' {
                break;
            }
            // Skip comments inside selector list too.
            if c == b'/' && self.src.get(self.pos + 1) == Some(&b'*') {
                self.skip_ws_and_comments();
                continue;
            }
            sel_buf.push(c as char);
            self.bump();
        }
        if self.peek() != Some(b'{') {
            return Err(ParseError::UnexpectedEof { line: start_line });
        }
        self.bump(); // {

        let selectors = parse_selector_list(sel_buf.trim());

        let mut declarations = Vec::new();
        loop {
            self.skip_ws_and_comments();
            match self.peek() {
                Some(b'}') => {
                    self.bump();
                    break;
                }
                None => return Err(ParseError::UnexpectedEof { line: self.line }),
                _ => {}
            }
            match self.parse_declaration() {
                Some(d) => declarations.push(d),
                None => {} // recovery
            }
        }

        if selectors.is_empty() {
            // Recover but don't drop the whole file.
            return Ok(None);
        }
        Ok(Some(Rule {
            selectors,
            declarations,
            source_order: order,
        }))
    }

    fn parse_declaration(&mut self) -> Option<Declaration> {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c == b':' || c == b'}' || c == b';' {
                break;
            }
            name.push(c as char);
            self.bump();
        }
        let name = name.trim().to_string();
        if name.is_empty() || self.peek() != Some(b':') {
            // Skip to next ';' or '}'.
            while let Some(c) = self.peek() {
                if c == b';' {
                    self.bump();
                    break;
                }
                if c == b'}' {
                    break;
                }
                self.bump();
            }
            return None;
        }
        self.bump(); // :

        let mut value = String::new();
        let mut in_str: Option<u8> = None;
        while let Some(c) = self.peek() {
            if let Some(q) = in_str {
                value.push(c as char);
                self.bump();
                if c == q {
                    in_str = None;
                }
                continue;
            }
            if c == b'"' || c == b'\'' {
                in_str = Some(c);
                value.push(c as char);
                self.bump();
                continue;
            }
            if c == b';' || c == b'}' {
                break;
            }
            value.push(c as char);
            self.bump();
        }
        if self.peek() == Some(b';') {
            self.bump();
        }

        let mut value = value.trim().to_string();
        let important = if let Some(stripped) = value.strip_suffix("!important") {
            value = stripped.trim_end().to_string();
            true
        } else {
            false
        };

        Some(Declaration {
            name,
            value: Value::Raw(value),
            important,
        })
    }
}

fn parse_selector_list(input: &str) -> Vec<Selector> {
    let mut out = Vec::new();
    // Split on top-level commas (no parens to worry about in our subset for now;
    // attribute brackets are handled).
    let mut depth: i32 = 0;
    let mut start = 0;
    let bytes = input.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        match *b {
            b'[' => depth += 1,
            b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                if let Some(sel) = parse_selector(input[start..i].trim()) {
                    out.push(sel);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < input.len() {
        if let Some(sel) = parse_selector(input[start..].trim()) {
            out.push(sel);
        }
    }
    out
}

fn parse_selector(input: &str) -> Option<Selector> {
    if input.is_empty() {
        return None;
    }
    // Tokenize into compound selectors and combinators.
    let mut parts: Vec<(SimpleSelector, Combinator)> = Vec::new();
    let mut cur = String::new();
    let mut pending_combinator: Option<Combinator> = None;
    let mut chars = input.chars().peekable();

    let flush =
        |cur: &mut String,
         parts: &mut Vec<(SimpleSelector, Combinator)>,
         pending: &mut Option<Combinator>| {
            let trimmed = cur.trim();
            if !trimmed.is_empty() {
                if let Some(prev_idx) = parts.len().checked_sub(1) {
                    if let Some(c) = pending.take() {
                        parts[prev_idx].1 = c;
                    } else {
                        // Implicit descendant if we have a previous and no
                        // combinator was explicitly set.
                        if parts[prev_idx].1 == Combinator::None {
                            parts[prev_idx].1 = Combinator::Descendant;
                        }
                    }
                }
                let simple = parse_simple_selector(trimmed)?;
                parts.push((simple, Combinator::None));
                cur.clear();
            } else if pending.is_some() {
                // dangling combinator without right side — invalid; drop.
                return None;
            }
            Some(())
        };

    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' => {
                chars.next();
                if !cur.trim().is_empty()
                    && flush(&mut cur, &mut parts, &mut pending_combinator).is_none()
                {
                    return None;
                }
            }
            '>' => {
                chars.next();
                if flush(&mut cur, &mut parts, &mut pending_combinator).is_none() {
                    return None;
                }
                pending_combinator = Some(Combinator::Child);
            }
            _ => {
                cur.push(c);
                chars.next();
            }
        }
    }
    if flush(&mut cur, &mut parts, &mut pending_combinator).is_none() {
        return None;
    }
    if parts.is_empty() {
        return None;
    }
    Some(Selector { parts })
}

fn parse_simple_selector(input: &str) -> Option<SimpleSelector> {
    let mut s = SimpleSelector::default();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'#' => {
                i += 1;
                let start = i;
                while i < bytes.len() && is_ident(bytes[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                s.id = Some(input[start..i].to_string());
            }
            b'.' => {
                i += 1;
                let start = i;
                while i < bytes.len() && is_ident(bytes[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                s.classes.push(input[start..i].to_string());
            }
            b'[' => {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != b']' {
                    i += 1;
                }
                if i >= bytes.len() {
                    return None;
                }
                let inner = &input[start..i];
                i += 1; // ]
                if let Some(eq) = inner.find('=') {
                    let name = inner[..eq].trim().to_string();
                    let mut val = inner[eq + 1..].trim().to_string();
                    if (val.starts_with('"') && val.ends_with('"'))
                        || (val.starts_with('\'') && val.ends_with('\''))
                    {
                        val = val[1..val.len() - 1].to_string();
                    }
                    s.attrs.push(AttrSelector {
                        name,
                        value: Some(val),
                    });
                } else {
                    s.attrs.push(AttrSelector {
                        name: inner.trim().to_string(),
                        value: None,
                    });
                }
            }
            b':' => {
                i += 1;
                let start = i;
                while i < bytes.len() && is_ident(bytes[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                s.pseudos.push(input[start..i].to_string());
            }
            _ => {
                // Unknown leading char (no element selectors in our subset).
                return None;
            }
        }
    }
    if s.is_empty() {
        return None;
    }
    Some(s)
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_id_class_attr_and_pseudo() {
        let sheet = parse("#composer.focused[data-mode=\"plan\"]:hover { color: red; }").unwrap();
        assert_eq!(sheet.rules.len(), 1);
        let sel = &sheet.rules[0].selectors[0];
        let simple = sel.key();
        assert_eq!(simple.id.as_deref(), Some("composer"));
        assert_eq!(simple.classes, vec!["focused".to_string()]);
        assert_eq!(simple.attrs[0].name, "data-mode");
        assert_eq!(simple.attrs[0].value.as_deref(), Some("plan"));
        assert_eq!(simple.pseudos, vec!["hover".to_string()]);
    }

    #[test]
    fn parses_descendant_and_child_combinators() {
        let sheet = parse("#status-line .segment > .label { color: blue; }").unwrap();
        let parts = &sheet.rules[0].selectors[0].parts;
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].1, Combinator::Descendant);
        assert_eq!(parts[1].1, Combinator::Child);
        assert_eq!(parts[2].1, Combinator::None);
    }

    #[test]
    fn parses_comma_groups() {
        let sheet = parse(".a, .b , .c { color: red; }").unwrap();
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selectors.len(), 3);
    }

    #[test]
    fn collects_root_variables() {
        let sheet = parse(":root { --accent: #ff8800; --muted: gray; }").unwrap();
        assert_eq!(sheet.rules.len(), 0);
        assert_eq!(
            sheet.variables,
            vec![
                ("accent".to_string(), "#ff8800".to_string()),
                ("muted".to_string(), "gray".to_string()),
            ]
        );
    }

    #[test]
    fn recovers_from_garbage_between_rules() {
        let sheet = parse(".a { color: red; } GARBAGE { ??? } .b { color: blue; }").unwrap();
        // The GARBAGE rule has no parseable selector; it should be dropped, the
        // rest should still parse.
        let class_lists: Vec<_> = sheet
            .rules
            .iter()
            .map(|r| r.selectors[0].key().classes.clone())
            .collect();
        assert!(class_lists.contains(&vec!["a".to_string()]));
        assert!(class_lists.contains(&vec!["b".to_string()]));
    }

    #[test]
    fn handles_important_marker() {
        let sheet = parse(".x { color: red !important; }").unwrap();
        assert!(sheet.rules[0].declarations[0].important);
    }

    #[test]
    fn rejects_oversize_input() {
        let huge = "a".repeat(SIZE_LIMIT_BYTES + 1);
        assert!(matches!(parse(&huge), Err(ParseError::TooLarge { .. })));
    }

    #[test]
    fn specificity_tuples() {
        let sheet = parse("#id .c[attr]:pseudo { color: red; } .a .b { color: red; }").unwrap();
        assert_eq!(sheet.rules[0].selectors[0].specificity(), (1, 3, 0));
        assert_eq!(sheet.rules[1].selectors[0].specificity(), (0, 2, 0));
    }
}
