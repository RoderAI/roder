//! Observed form structure distinguishes an unrelated Go button from submission.
use crate::jev_prompt;
use serde_json::json;
#[tokio::test]
async fn jev_evidence_names_form_ownership_and_disabled_controls() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let actions = observation["actions"].as_array().unwrap();
    let search = actions
        .iter()
        .find(|a| a["kind"] == "fill" && a["label"] == "Search")
        .unwrap();
    assert!(search["form"]["id"].is_number() || search["form"]["id"].is_string());
    assert_eq!(search["form"]["submit_buttons"], json!([]));
    assert!(actions.iter().find(|a| a["label"] == "Go").unwrap()["form"].is_null());
    let (mut request, _, _) = crate::decide::request_body(
        &observation,
        "Search for boots",
        &[],
        "jev-latest",
        chrono::Local::now().date_naive(),
    );
    let baseline = request.clone();
    jev_prompt::rewrite(
        &mut request,
        jev_prompt::Profile::Baseline,
        &[],
        &observation,
    );
    assert_eq!(request, baseline);
    jev_prompt::rewrite(&mut request, jev_prompt::Profile::Form, &[], &observation);
    assert_eq!(request["state"]["elements"][0]["form"], search["form"]);
    let go = actions.iter().find(|a| a["label"] == "Go").unwrap();
    page.evaluate("document.getElementById('go').setAttribute('form','search'); true")
        .await
        .unwrap();
    assert!(
        !page.fresh(&observation, Some(go)).await.unwrap(),
        "Changed form ownership must invalidate a selected action"
    );
    page.evaluate(
        "document.querySelector('#search input').setAttribute('form','missing-form'); true",
    )
    .await
    .unwrap();
    let detached = page.observe().await.unwrap();
    assert!(
        detached["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["kind"] == "fill" && a["label"] == "Search")
            .unwrap()["form"]
            .is_null(),
        "Explicit invalid form association must not inherit the ancestor form"
    );
    page.close().await.unwrap();
    let mut page = harness.open("terms.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    assert!(
        observation["disabled_controls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["label"] == "I agree")
    );
    assert!(
        !observation["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["kind"] == "click" && a["label"] == "I agree")
    );
    page.close().await.unwrap();
}

#[test]
fn production_jev_profile_keeps_candidates_and_safety_questions() {
    let observation = json!({"actions":[{"kind":"click","label":"Submit","node":1,"form":null}], "disabled_controls":[{"label":"Publish","role":"button"}]});
    let history = (0..12).map(|i|json!({"action":format!("Step {i}"),"context":"Draft A","page_changed":true,"effect":"verbose effect","text_added":["previous text"]})).collect::<Vec<_>>();
    let (mut request, space, _) = crate::decide::request_body(
        &observation,
        "Save the draft",
        &history,
        "jev-latest",
        chrono::Local::now().date_naive(),
    );
    let asked = crate::irreversible::add_questions(&mut request, &space, "Save the draft");
    assert_eq!(asked.len(), 1);
    let gate = &asked[0].key;
    assert_eq!(request["questions"][gate]["type"], "noul");
    let original = request.clone();
    jev_prompt::rewrite(
        &mut request,
        jev_prompt::Profile::Workflow,
        &history,
        &observation,
    );
    assert_eq!(request["questions"][gate], original["questions"][gate]);
    assert_eq!(
        request["questions"]["operation"]["criteria"],
        original["questions"]["operation"]["criteria"]
    );
    assert_eq!(
        request["questions"]["click_target"]["criteria"]["1"]["element"],
        original["questions"]["click_target"]["criteria"]["1"]["element"]
    );
    let recent = request["state"]["recent_actions"].as_array().unwrap();
    assert_eq!(recent.len(), 10);
    assert_eq!(recent[0]["action"], "Step 2");
    assert_eq!(recent[0]["context"], "Draft A");
    assert!(recent[0].get("effect").is_none());
    assert!(recent[0].get("text_added").is_none());
    assert_eq!(
        request["state"]["disabled_controls"],
        observation["disabled_controls"]
    );
    assert!(
        request["questions"]["operation"]["instructions"]["rules"]
            .as_str()
            .unwrap()
            .contains("untrusted")
    );
}

#[test]
fn dom_profile_uses_observed_structure_preserves_gate_and_candidate_ids() {
    let observation = json!({"actions":[{"kind":"click","node":12,"label":"Submit","form":null}],"dom_snapshot":"- main\n  - button \"Submit\" [node=12]"});
    let (mut request, space, _) = crate::decide::request_body(
        &observation,
        "Save draft",
        &[],
        "jev-preview",
        chrono::Local::now().date_naive(),
    );
    let asked = crate::irreversible::add_questions(&mut request, &space, "Save draft");
    assert_eq!(asked.len(), 1);
    let gate = &asked[0].key;
    assert_eq!(request["questions"][gate]["type"], "noul");
    let before = request.clone();
    jev_prompt::rewrite(&mut request, jev_prompt::Profile::Dom, &[], &observation);
    assert_eq!(
        request["state"]["dom_snapshot"],
        observation["dom_snapshot"]
    );
    assert_eq!(request["questions"][gate], before["questions"][gate]);
    assert_eq!(
        request["questions"]["click_target"]["criteria"]["1"]["element"],
        before["questions"]["click_target"]["criteria"]["1"]["element"]
    );
    assert_eq!(
        request["questions"]["operation"]["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        before["questions"]["operation"]["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>()
    );
    assert_eq!(
        request["questions"]["operation"]["criteria"]["CLICK"]["available_targets"],
        request["questions"]["click_target"]["criteria"]
    );
}
