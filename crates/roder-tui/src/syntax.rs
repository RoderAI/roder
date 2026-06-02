use std::path::Path;

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum SyntaxKind {
    Plain,
    Keyword,
    String,
    Number,
    Comment,
    Type,
    Function,
    Macro,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum SyntaxLanguage {
    CLike,
    Rust,
    Python,
    Shell,
    Toml,
    Markdown,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct SyntaxTheme {
    pub base: Color,
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub ty: Color,
    pub function: Color,
    pub mac: Color,
    pub bg: Option<Color>,
}

pub(crate) fn language_for_path(path: &Path) -> Option<SyntaxLanguage> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    language_for_extension(&extension)
}

pub(crate) fn language_for_extension(extension: &str) -> Option<SyntaxLanguage> {
    match extension {
        "rs" => Some(SyntaxLanguage::Rust),
        "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" | "hxx" | "go" | "java" | "js" | "jsx" | "ts"
        | "tsx" | "cs" | "swift" | "kt" | "kts" => Some(SyntaxLanguage::CLike),
        "py" | "pyw" => Some(SyntaxLanguage::Python),
        "sh" | "bash" | "zsh" | "fish" => Some(SyntaxLanguage::Shell),
        "toml" => Some(SyntaxLanguage::Toml),
        "md" | "markdown" => Some(SyntaxLanguage::Markdown),
        _ => None,
    }
}

pub(crate) fn highlight_code(
    text: &str,
    language: Option<SyntaxLanguage>,
    theme: SyntaxTheme,
) -> Vec<Span<'static>> {
    let Some(language) = language else {
        return vec![Span::styled(
            text.to_string(),
            Style::default().fg(theme.base),
        )];
    };
    tokenize_code(text, language)
        .into_iter()
        .map(|(token, kind)| Span::styled(token, syntax_style(kind, theme)))
        .collect()
}

pub(crate) fn padded_highlighted_code(
    text: &str,
    width: usize,
    language: Option<SyntaxLanguage>,
    theme: SyntaxTheme,
) -> Vec<Span<'static>> {
    let mut spans = highlight_code(text, language, theme);
    let width_used = text.chars().count();
    if width_used < width {
        spans.push(Span::styled(
            " ".repeat(width - width_used),
            Style::default().fg(theme.base),
        ));
    }
    spans
}

fn syntax_style(kind: SyntaxKind, theme: SyntaxTheme) -> Style {
    let color = match kind {
        SyntaxKind::Plain => theme.base,
        SyntaxKind::Keyword => theme.keyword,
        SyntaxKind::String => theme.string,
        SyntaxKind::Number => theme.number,
        SyntaxKind::Comment => theme.comment,
        SyntaxKind::Type => theme.ty,
        SyntaxKind::Function => theme.function,
        SyntaxKind::Macro => theme.mac,
    };
    let mut style = Style::default().fg(color);
    if let Some(bg) = theme.bg {
        style = style.bg(bg);
    }
    if matches!(kind, SyntaxKind::Keyword | SyntaxKind::Type) {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

fn tokenize_code(text: &str, language: SyntaxLanguage) -> Vec<(String, SyntaxKind)> {
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        if let Some(comment_len) = line_comment_len(rest, language) {
            tokens.push((rest[..comment_len].to_string(), SyntaxKind::Comment));
            index += comment_len;
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == '"' || ch == '\'' || (language == SyntaxLanguage::Rust && ch == '`') {
            let len = quoted_len(rest, ch);
            tokens.push((rest[..len].to_string(), SyntaxKind::String));
            index += len;
        } else if ch.is_ascii_digit() {
            let len = number_len(rest);
            tokens.push((rest[..len].to_string(), SyntaxKind::Number));
            index += len;
        } else if is_ident_start(ch) {
            let len = ident_len(rest);
            let token = &rest[..len];
            let kind = if is_keyword(token, language) {
                SyntaxKind::Keyword
            } else if is_type_like(token, language) {
                SyntaxKind::Type
            } else if rest[len..].starts_with('!') && language == SyntaxLanguage::Rust {
                SyntaxKind::Macro
            } else if next_non_ws_starts_with(&rest[len..], '(') {
                SyntaxKind::Function
            } else {
                SyntaxKind::Plain
            };
            tokens.push((token.to_string(), kind));
            index += len;
        } else {
            let len = ch.len_utf8();
            tokens.push((rest[..len].to_string(), SyntaxKind::Plain));
            index += len;
        }
    }
    tokens
}

fn line_comment_len(rest: &str, language: SyntaxLanguage) -> Option<usize> {
    match language {
        SyntaxLanguage::Python | SyntaxLanguage::Shell | SyntaxLanguage::Toml => {
            rest.starts_with('#').then_some(rest.len())
        }
        SyntaxLanguage::Markdown => rest.trim_start().starts_with("<!--").then_some(rest.len()),
        SyntaxLanguage::CLike | SyntaxLanguage::Rust => {
            (rest.starts_with("//") || rest.starts_with("/*")).then_some(rest.len())
        }
    }
}

fn quoted_len(rest: &str, quote: char) -> usize {
    let mut escaped = false;
    for (offset, ch) in rest.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return offset + ch.len_utf8();
        }
    }
    rest.len()
}

fn number_len(rest: &str) -> usize {
    rest.char_indices()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
        .last()
        .map(|(offset, ch)| offset + ch.len_utf8())
        .unwrap_or(0)
}

fn ident_len(rest: &str) -> usize {
    rest.char_indices()
        .take_while(|(_, ch)| is_ident_continue(*ch))
        .last()
        .map(|(offset, ch)| offset + ch.len_utf8())
        .unwrap_or(0)
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch == '-' || ch.is_ascii_alphanumeric()
}

fn next_non_ws_starts_with(text: &str, needle: char) -> bool {
    text.trim_start().starts_with(needle)
}

fn is_type_like(token: &str, language: SyntaxLanguage) -> bool {
    match language {
        SyntaxLanguage::Rust => matches!(
            token,
            "Self"
                | "String"
                | "Vec"
                | "Option"
                | "Result"
                | "Box"
                | "Arc"
                | "HashMap"
                | "HashSet"
                | "usize"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "bool"
                | "str"
                | "char"
        ),
        _ => token.chars().next().is_some_and(char::is_uppercase),
    }
}

fn is_keyword(token: &str, language: SyntaxLanguage) -> bool {
    match language {
        SyntaxLanguage::Rust => matches!(
            token,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
        ),
        SyntaxLanguage::CLike => matches!(
            token,
            "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "do"
                | "else"
                | "enum"
                | "export"
                | "extends"
                | "false"
                | "final"
                | "for"
                | "from"
                | "func"
                | "function"
                | "go"
                | "if"
                | "import"
                | "interface"
                | "let"
                | "new"
                | "null"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "return"
                | "static"
                | "struct"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "type"
                | "var"
                | "void"
                | "while"
        ),
        SyntaxLanguage::Python => matches!(
            token,
            "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "False"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "None"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "True"
                | "try"
                | "while"
                | "with"
                | "yield"
        ),
        SyntaxLanguage::Shell => matches!(
            token,
            "case"
                | "do"
                | "done"
                | "elif"
                | "else"
                | "esac"
                | "fi"
                | "for"
                | "function"
                | "if"
                | "in"
                | "select"
                | "then"
                | "until"
                | "while"
        ),
        SyntaxLanguage::Toml => matches!(token, "true" | "false"),
        SyntaxLanguage::Markdown => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_highlighting_marks_keywords_strings_and_macros() {
        let theme = SyntaxTheme {
            base: Color::Reset,
            keyword: Color::Indexed(1),
            string: Color::Indexed(2),
            number: Color::Indexed(3),
            comment: Color::Indexed(4),
            ty: Color::Indexed(5),
            function: Color::Indexed(6),
            mac: Color::Indexed(7),
            bg: None,
        };
        let spans = highlight_code(
            r#"let value: String = format!("hi") // note"#,
            Some(SyntaxLanguage::Rust),
            theme,
        );

        assert!(
            spans
                .iter()
                .any(|span| span.content.as_ref() == "let" && span.style.fg == Some(theme.keyword))
        );
        assert!(
            spans
                .iter()
                .any(|span| span.content.as_ref() == "String" && span.style.fg == Some(theme.ty))
        );
        assert!(
            spans
                .iter()
                .any(|span| span.content.as_ref() == "format" && span.style.fg == Some(theme.mac))
        );
        assert!(
            spans
                .iter()
                .any(|span| span.content.as_ref() == r#""hi""#
                    && span.style.fg == Some(theme.string))
        );
        assert!(
            spans
                .iter()
                .any(|span| span.content.as_ref() == "// note"
                    && span.style.fg == Some(theme.comment))
        );
    }
}
