use std::collections::BTreeMap;

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateContext {
    pub arguments: String,
    pub includes: BTreeMap<String, String>,
}

pub fn render_template(template: &str, context: &TemplateContext) -> Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            bail!("template expression is missing closing `}}}}`");
        };
        let expression = after_start[..end].trim();
        out.push_str(&resolve_expression(expression, context)?);
        rest = &after_start[end + 2..];
    }

    out.push_str(rest);
    Ok(out)
}

fn resolve_expression(expression: &str, context: &TemplateContext) -> Result<String> {
    if expression == "arguments" {
        return Ok(context.arguments.clone());
    }
    if let Some(default) = parse_arguments_default(expression)? {
        return Ok(if context.arguments.trim().is_empty() {
            default
        } else {
            context.arguments.clone()
        });
    }
    if let Some(include_key) = expression.strip_prefix("include.") {
        return context.includes.get(include_key).cloned().ok_or_else(|| {
            anyhow::anyhow!("unknown include template key `include.{include_key}`")
        });
    }
    bail!("unsupported template expression `{{{{{expression}}}}}`")
}

fn parse_arguments_default(expression: &str) -> Result<Option<String>> {
    let Some(inner) = expression
        .strip_prefix("arguments|default(")
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Ok(None);
    };
    parse_quoted_string(inner.trim()).map(Some)
}

fn parse_quoted_string(value: &str) -> Result<String> {
    if value.len() < 2 {
        bail!("default value must be a quoted string");
    }
    let bytes = value.as_bytes();
    let quote = bytes[0];
    if !matches!(quote, b'"' | b'\'') || bytes[value.len() - 1] != quote {
        bail!("default value must be a quoted string");
    }
    Ok(value[1..value.len() - 1].to_string())
}

pub fn include_template_key(kind: &str, id: &str) -> String {
    format!("{kind}.{id}")
}

pub fn default_include_id(value: &str, fallback: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        fallback.to_string()
    } else {
        out.to_string()
    }
}

pub fn include_reference(command_name: &str, kind: &str, id: &str) -> String {
    format!("[context:command.{command_name}.{kind}.{id}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_arguments_and_default() {
        let context = TemplateContext {
            arguments: "api".to_string(),
            includes: BTreeMap::new(),
        };
        assert_eq!(
            render_template("Run {{arguments}}", &context).unwrap(),
            "Run api"
        );

        let context = TemplateContext {
            arguments: String::new(),
            includes: BTreeMap::new(),
        };
        assert_eq!(
            render_template(r#"Run {{arguments|default("all")}}"#, &context).unwrap(),
            "Run all"
        );
    }

    #[test]
    fn rejects_unknown_expression() {
        let context = TemplateContext {
            arguments: String::new(),
            includes: BTreeMap::new(),
        };
        let err = render_template("{{missing}}", &context)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported template expression"), "{err}");
    }
}
