use std::collections::BTreeSet;

use roder_protocol::methods::app_server_method_specs;

#[test]
fn method_manifest_matches_handlers() {
    let handled = handled_methods_from_source();
    let manifest = app_server_method_specs()
        .iter()
        .map(|spec| spec.method)
        .collect::<BTreeSet<_>>();

    let missing = handled.difference(&manifest).copied().collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "methods missing from manifest: {missing:?}"
    );
}

fn handled_methods_from_source() -> BTreeSet<&'static str> {
    let source = include_str!("server.rs");
    let handle_request = source
        .split("pub async fn handle_request")
        .nth(1)
        .expect("handle_request source");
    let match_body = handle_request
        .split("fn invalid_params")
        .next()
        .expect("handle_request body");

    let mut methods = BTreeSet::new();
    for line in match_body.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix('"') else {
            continue;
        };
        let Some(end) = rest.find('"') else {
            continue;
        };
        let method = &rest[..end];
        let after_literal = rest[end + 1..].trim_start();
        if after_literal.starts_with("=>") {
            methods.insert(method);
        }
    }
    methods
}
