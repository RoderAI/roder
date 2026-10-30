use super::*;
#[test]
fn london_suggestions_keep_hierarchy_focus_and_refs_without_unchanged_content() {
    let before = json!({"document":"doc","url":"https://flight.test/","text":"Header\nFrom SFO","text_hash":1,"rows":[
        {"id":"nav","role":"navigation","label":"Header"},
        {"id":"form","role":"form","label":"Flights"},
        {"id":"to","parent":"form","role":"combobox","label":"To","ref":"e1","value":""}
    ]});
    let mut after = before.clone();
    after["text"] = json!("Header\nFrom SFO\nLondon all airports");
    after["text_hash"] = json!(2);
    after["rows"][2]["value"] = json!("London");
    after["rows"][2]["focused"] = json!(true);
    after["rows"][2]["expanded"] = json!(true);
    after["rows"].as_array_mut().unwrap().extend([
        json!({"id":"list","parent":"form","role":"listbox","label":"To suggestions"}),
        json!({"id":"option","parent":"list","role":"option","ref":"e2","label":"London all airports"})
    ]);
    let diff = changes(&after, Some(&before));
    let text = render(&diff, 100);
    assert_eq!(diff["mode"], "diff");
    assert!(!text.contains("Header"));
    assert!(
        text.contains("e1 combobox \"To\" = \"London\" [expanded] [focused]"),
        "{text}"
    );
    assert!(
        text.contains("e2 option \"London all airports\" (in listbox \"To suggestions\")"),
        "{text}"
    );
    assert_eq!(diff["text"], "+ London all airports");
    after["document"] = json!("new");
    assert_eq!(changes(&after, Some(&before))["mode"], "full");
    after["document"] = json!("doc");
    after["limited"] = json!(true);
    assert_eq!(changes(&after, Some(&before))["mode"], "full");
}
#[test]
fn truncated_text_never_claims_that_unobserved_lines_were_removed() {
    assert_eq!(text_delta("A\nB", "A\nB\nC", false), "");
    assert_eq!(text_delta("A\nB", "A\nB\nC", true), "- C");
    assert_eq!(text_delta("A\nA", "A", true), "+ A");
}
