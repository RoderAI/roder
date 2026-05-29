use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphPatchOperation {
    pub op: String,
    #[serde(default)]
    pub node: Option<String>,
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default)]
    pub expect: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub edge: Option<String>,
    #[serde(default)]
    pub order: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type", default)]
    pub ty: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub line: Option<u64>,
    #[serde(default)]
    pub column: Option<u64>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub public: Option<bool>,
    #[serde(default)]
    pub mutable: Option<bool>,
    #[serde(default)]
    pub static_: Option<bool>,
    #[serde(default)]
    pub fallible: Option<bool>,
    #[serde(rename = "exportC", default)]
    pub export_c: Option<bool>,
}

pub fn build_patch_text(
    graph_hash: &str,
    operations: &[GraphPatchOperation],
) -> anyhow::Result<String> {
    validate_graph_hash(graph_hash)?;
    if operations.is_empty() {
        bail!("zerolang_edit requires at least one graph patch operation");
    }
    let mut lines = vec![
        "zero-program-graph-patch v1".to_string(),
        format!("expect graphHash {}", quote(graph_hash)?),
    ];
    for operation in operations {
        lines.push(operation_line(operation)?);
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}

fn validate_graph_hash(value: &str) -> anyhow::Result<()> {
    let Some(hex) = value.strip_prefix("graph:") else {
        bail!("graphHash must start with graph:");
    };
    if hex.len() != 16 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("graphHash must match graph:<16 lowercase hex digits>");
    }
    Ok(())
}

fn operation_line(operation: &GraphPatchOperation) -> anyhow::Result<String> {
    match operation.op.as_str() {
        "set" => line(
            "set",
            vec![
                required("node", &operation.node)?,
                required("field", &operation.field)?,
                optional("expect", &operation.expect)?,
                required("value", &operation.value)?,
            ],
        ),
        "rename" => line(
            "rename",
            vec![
                required("node", &operation.node)?,
                optional("expect", &operation.expect)?,
                required("value", &operation.value)?,
            ],
        ),
        "insert" => line(
            "insert",
            common_node_attrs(
                vec![
                    required("node", &operation.node)?,
                    required("kind", &operation.kind)?,
                    required("parent", &operation.parent)?,
                    required("edge", &operation.edge)?,
                    required_u64("order", operation.order)?,
                ],
                operation,
            )?,
        ),
        "insertEdge" => line(
            "insertEdge",
            vec![
                required("from", &operation.from)?,
                required("to", &operation.to)?,
                required("edge", &operation.edge)?,
                required("target", &operation.target)?,
                required_u64("order", operation.order)?,
            ],
        ),
        "replace" => line(
            "replace",
            common_node_attrs(
                vec![
                    required("node", &operation.node)?,
                    optional("expect", &operation.expect)?,
                    optional("kind", &operation.kind)?,
                ],
                operation,
            )?,
        ),
        "delete" => line(
            "delete",
            vec![
                required("node", &operation.node)?,
                optional("expect", &operation.expect)?,
            ],
        ),
        other => bail!(
            "unsupported graph patch operation {other:?}; expected set, rename, insert, insertEdge, replace, or delete"
        ),
    }
}

fn common_node_attrs(
    mut attrs: Vec<Option<String>>,
    operation: &GraphPatchOperation,
) -> anyhow::Result<Vec<Option<String>>> {
    attrs.extend([
        optional("name", &operation.name)?,
        optional("type", &operation.ty)?,
        optional("value", &operation.value)?,
        optional("path", &operation.path)?,
        optional_u64("line", operation.line)?,
        optional_u64("column", operation.column)?,
        optional_bool("public", operation.public)?,
        optional_bool("mutable", operation.mutable)?,
        optional_bool("static", operation.static_)?,
        optional_bool("fallible", operation.fallible)?,
        optional_bool("exportC", operation.export_c)?,
    ]);
    Ok(attrs)
}

fn line(op: &str, attrs: Vec<Option<String>>) -> anyhow::Result<String> {
    let attrs = attrs.into_iter().flatten().collect::<Vec<_>>();
    Ok(format!("{op} {}", attrs.join(" ")))
}

fn required(name: &str, value: &Option<String>) -> anyhow::Result<Option<String>> {
    let value = value
        .as_deref()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("graph patch operation requires {name}"))?;
    Ok(Some(format!("{name}={}", quote(value)?)))
}

fn optional(name: &str, value: &Option<String>) -> anyhow::Result<Option<String>> {
    value
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| Ok(format!("{name}={}", quote(value)?)))
        .transpose()
}

fn required_u64(name: &str, value: Option<u64>) -> anyhow::Result<Option<String>> {
    let value = value.with_context(|| format!("graph patch operation requires {name}"))?;
    Ok(Some(format!("{name}={}", quote(&value.to_string())?)))
}

fn optional_u64(name: &str, value: Option<u64>) -> anyhow::Result<Option<String>> {
    value
        .map(|value| Ok(format!("{name}={}", quote(&value.to_string())?)))
        .transpose()
}

fn optional_bool(name: &str, value: Option<bool>) -> anyhow::Result<Option<String>> {
    value
        .map(|value| {
            Ok(format!(
                "{name}={}",
                quote(if value { "true" } else { "false" })?
            ))
        })
        .transpose()
}

fn quote(value: &str) -> anyhow::Result<String> {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\0' => bail!("NUL bytes are not valid ProgramGraph patch text"),
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch < ' ' => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch if (ch as u32) <= 0xff && !ch.is_ascii() => {
                escaped.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    Ok(escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_checked_patch_text_with_escaped_values() {
        let text = build_patch_text(
            "graph:f76987e99677f1b3",
            &[GraphPatchOperation {
                op: "set".to_string(),
                node: Some("#610c78bf".to_string()),
                field: Some("value".to_string()),
                expect: Some("hello\n".to_string()),
                value: Some("hello \"agent\"\\\n".to_string()),
                kind: None,
                parent: None,
                edge: None,
                order: None,
                name: None,
                ty: None,
                path: None,
                line: None,
                column: None,
                from: None,
                to: None,
                target: None,
                public: None,
                mutable: None,
                static_: None,
                fallible: None,
                export_c: None,
            }],
        )
        .unwrap();

        assert!(text.starts_with("zero-program-graph-patch v1\n"));
        assert!(text.contains("expect graphHash \"graph:f76987e99677f1b3\""));
        assert!(text.contains(
            "set node=\"#610c78bf\" field=\"value\" expect=\"hello\\n\" value=\"hello \\\"agent\\\"\\\\\\n\""
        ));
    }

    #[test]
    fn rejects_stale_guard_without_graph_hash_prefix() {
        let err = build_patch_text("f76987e99677f1b3", &[]).unwrap_err();

        assert!(err.to_string().contains("graphHash must start with graph:"));
    }
}
