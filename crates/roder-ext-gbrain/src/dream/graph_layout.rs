use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphIdParts<'a> {
    pub scope: Option<&'a str>,
    pub kind: &'a str,
    pub label: &'a str,
}

pub fn normalize_graph_id(parts: GraphIdParts<'_>) -> String {
    let mut segments = Vec::with_capacity(3);
    if let Some(scope) = parts.scope {
        let scope = normalize_segment(scope);
        if !scope.is_empty() {
            segments.push(scope);
        }
    }
    segments.push(normalize_segment(parts.kind));
    segments.push(normalize_segment(parts.label));
    segments
        .into_iter()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(":")
}

pub fn validate_graph_edge_endpoints(
    source_node_id: &str,
    target_node_id: &str,
) -> Result<(), String> {
    if source_node_id.trim().is_empty() {
        return Err("source node id is required".to_string());
    }
    if target_node_id.trim().is_empty() {
        return Err("target node id is required".to_string());
    }
    if source_node_id == target_node_id {
        return Err("graph edge endpoints must be distinct".to_string());
    }
    Ok(())
}

fn normalize_segment(input: &str) -> String {
    let mut out = String::new();
    let mut pending_separator = false;
    for ch in input.nfkc().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            if pending_separator && !out.is_empty() {
                out.push('_');
            }
            out.push(ch);
            pending_separator = false;
        } else {
            pending_separator = true;
        }
    }
    out.trim_matches('_').to_string()
}
