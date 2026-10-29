//! Semantic DOM evidence uses the real composed page and never field secrets.
use crate::secret::Secrets;
use serde_json::json;

#[tokio::test]
async fn semantic_dom_keeps_hierarchy_state_refs_and_scrubs_secrets() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    page.evaluate_async(r#"document.body.innerHTML = `
      <nav aria-label="Primary"><a href="/home">Home</a></nav>
      <main><h1>Account<span hidden>hidden-heading</span><textarea style="-webkit-text-security:disc" hidden>heading-secret</textarea></h1><form aria-label="Profile">
        <label>Name<input value="Ada"></label><label>Country<select><option>Germany</option><option selected>Chile</option></select></label>
        <label>Password<input type="password" value="prefilled-secret"></label>
        <textarea aria-label="Private" style="-webkit-text-security:disc">masked-initial</textarea>
        <button disabled>Save</button><button aria-expanded="false">More</button>
      </form><div role="status">Saved account</div>
      <p hidden>hidden-evidence</p><div id="host"></div>
      <iframe srcdoc='<h2>Frame heading</h2><button>Frame action</button>'></iframe></main>`;
      document.querySelector('#host').attachShadow({mode:'open'}).innerHTML='<section aria-label="Shadow"><button>Shadow action</button></section>';
      new Promise(resolve=>document.querySelector('iframe').onload=()=>resolve(true))"#).await.unwrap();
    let observation = page.observe().await.unwrap();
    let dom = observation["dom_snapshot"].as_str().unwrap();
    for expected in [
        "navigation \"Primary\"",
        "- main",
        "heading \"Account\" [level=1]",
        "form \"Profile\"",
        "textbox \"Name\"",
        "= \"Ada\"",
        "combobox \"Country\"",
        "= \"Chile\"",
        "button \"Save\" [disabled]",
        "[collapsed]",
        "status",
        "Saved account",
        "Shadow action",
        "Frame heading",
        "Frame action",
    ] {
        assert!(dom.contains(expected), "Missing {expected}: {dom}");
    }
    for secret in [
        "prefilled-secret",
        "masked-initial",
        "hidden-evidence",
        "hidden-heading",
        "heading-secret",
    ] {
        assert!(!dom.contains(secret), "{dom}");
    }
    let more = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["label"] == "More")
        .unwrap();
    assert!(dom.contains(&format!("{}", more["ref"].as_str().unwrap())));
    let next = page.observe().await.unwrap();
    assert_eq!(next["dom_snapshot"], observation["dom_snapshot"]);
    page.evaluate("document.querySelector('button[aria-expanded]').setAttribute('aria-expanded','true'); true").await.unwrap();
    assert!(!page.fresh(&observation, Some(more)).await.unwrap());
    let mut echo = json!({"dom_snapshot":"- status: echoed-secret", "actions":[]});
    let mut secrets = Secrets::default();
    secrets.remember("echoed-secret");
    secrets.scrub_observation(&mut echo);
    assert_eq!(echo["dom_snapshot"], "- status: [secret]");
}

#[tokio::test]
async fn semantic_dom_caps_large_pages_and_marks_omissions() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    page.evaluate("document.body.innerHTML='<main>'+Array.from({length:300},(_,i)=>'<p>Line '+i+'</p>').join('')+'</main>'; true").await.unwrap();
    let observed = page.observe().await.unwrap();
    let dom = observed["dom_snapshot"].as_str().unwrap();
    assert!(dom.len() <= 6100, "{}", dom.len());
    assert!(dom.lines().count() <= 101);
    // Offscreen paragraphs are intentionally absent; force visible records to
    // exercise the cap independently of viewport height.
    page.evaluate("document.querySelectorAll('p').forEach(p=>p.style.cssText='position:absolute;top:0'); true").await.unwrap();
    let observed = page.observe().await.unwrap();
    assert!(
        observed["dom_snapshot"]
            .as_str()
            .unwrap()
            .contains("snapshot limited")
    );
}

#[tokio::test]
async fn semantic_dom_excludes_text_clipped_inside_a_scroll_panel() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    page.evaluate(r#"document.body.innerHTML='<main><section aria-label="Terms" style="height:60px;overflow:auto"><p>Visible introduction</p><div style="height:500px"></div><p>Hidden conclusion</p></section></main>'; true"#).await.unwrap();
    let first = page.observe().await.unwrap();
    let dom = first["dom_snapshot"].as_str().unwrap();
    assert!(dom.contains("Visible introduction"), "{dom}");
    assert!(!dom.contains("Hidden conclusion"), "{dom}");
    page.evaluate(
        "const panel=document.querySelector('section'); panel.scrollTop=panel.scrollHeight; true",
    )
    .await
    .unwrap();
    let last = page.observe().await.unwrap();
    let dom = last["dom_snapshot"].as_str().unwrap();
    assert!(dom.contains("Hidden conclusion"), "{dom}");
    assert!(!dom.contains("Visible introduction"), "{dom}");
}

#[tokio::test]
async fn semantic_dom_maps_controls_inside_headings_and_styled_toggle_state() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    page.evaluate(r#"document.body.innerHTML='<h2>Account <a href="/home">Open profile</a><span><button>Expand</button></span></h2><label><input type="checkbox" checked style="position:absolute;opacity:0;width:0;height:0">Notifications</label><label><input type="radio" checked style="position:absolute;opacity:0;width:0;height:0">Daily</label>'; true"#).await.unwrap();
    let observation = page.observe().await.unwrap();
    let dom = observation["dom_snapshot"].as_str().unwrap();
    for label in ["Open profile", "Expand", "Notifications", "Daily"] {
        let action = observation["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["label"] == label)
            .unwrap();
        let line = dom
            .lines()
            .find(|line| line.contains(&format!("{}", action["ref"].as_str().unwrap())))
            .unwrap_or_else(|| panic!("Missing {label}: {dom}"));
        if matches!(label, "Notifications" | "Daily") {
            assert!(line.contains("[checked]"), "{line}");
        }
    }
    assert!(!dom.contains("text \"Account"), "{dom}");
}

#[tokio::test]
async fn offscreen_dom_changes_do_not_invalidate_a_page_level_decision() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    page.evaluate(r#"document.body.innerHTML='<button>Visible action</button><div style="height:2000px"></div><button id="far">Far action</button>'; true"#).await.unwrap();
    let observation = page.observe().await.unwrap();
    let far = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["label"] == "Far action")
        .unwrap();
    assert_eq!(far["offscreen"], true);
    page.evaluate("document.querySelector('#far').textContent='Changed far action'; true")
        .await
        .unwrap();
    let updated = page.observe().await.unwrap();
    assert_eq!(updated["dom_snapshot"], observation["dom_snapshot"]);
    assert!(page.fresh(&observation, None).await.unwrap());
    assert!(!page.fresh(&observation, Some(far)).await.unwrap());
}

#[tokio::test]
async fn semantic_dom_scrubs_complete_escaped_echoes_before_prompt_or_digest() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    let secret = "ab\"cd\\ef  gh\nij";
    let long = format!("long-secret-prefix-{}", "x".repeat(200));
    page.evaluate(&format!("document.body.innerHTML='<main><p id=short></p><p id=long></p><button>Save</button></main>'; document.querySelector('#short').textContent={}; document.querySelector('#long').textContent={}; true", serde_json::to_string(secret).unwrap(),serde_json::to_string(&long).unwrap())).await.unwrap();
    let mut observation = page.observe().await.unwrap();
    let escaped = serde_json::to_string(&secret).unwrap();
    assert!(
        serde_json::to_string(&observation["snapshot"])
            .unwrap()
            .contains(&escaped)
    );
    assert!(
        !observation["dom_snapshot"]
            .as_str()
            .unwrap()
            .contains("long-secret-prefix")
    );
    assert!(
        observation["dom_snapshot"]
            .as_str()
            .unwrap()
            .contains("button \"Save\"")
    );
    let mut secrets = Secrets::default();
    secrets.remember(secret);
    secrets.remember(&long);
    secrets.scrub_observation(&mut observation);
    assert!(
        observation["dom_snapshot"]
            .as_str()
            .unwrap()
            .contains("[secret]")
    );
    let (mut request, _, _) = crate::decide::request_body(
        &observation,
        "Inspect result",
        &[],
        "jev-preview",
        chrono::Local::now().date_naive(),
    );
    crate::jev_prompt::rewrite(
        &mut request,
        crate::jev_prompt::Profile::Dom,
        &[],
        &observation,
    );
    assert!(!request.to_string().contains("ab\\\"cd"));
    assert!(!request.to_string().contains(&"x".repeat(50)));
    let data = json!({"status":"done", "visible_text":"Result", "page":{"dom_snapshot":observation["dom_snapshot"]}});
    let text = crate::report::tool_text(&mut data.clone());
    assert!(!text.contains(&"x".repeat(50)));
}

#[tokio::test]
async fn oversized_dom_records_do_not_hide_later_controls() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    page.evaluate("document.body.innerHTML='<main><p></p><div>Odd role</div><button>Continue</button></main>'; document.querySelector('p').textContent='x'.repeat(8000); document.querySelector('div').setAttribute('role','y'.repeat(8000)); true").await.unwrap();
    let observed = page.observe().await.unwrap();
    let dom = observed["dom_snapshot"].as_str().unwrap();
    let button = observed["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["label"] == "Continue")
        .unwrap();
    assert!(
        dom.contains(&format!(
            "{} button \"Continue\"",
            button["ref"].as_str().unwrap()
        )),
        "{dom}"
    );
    assert!(dom.contains("[long text omitted]"), "{dom}");
    assert!(dom.contains("[oversized DOM record omitted]"), "{dom}");
    assert!(dom.contains("snapshot limited"));
    assert!(dom.len() <= 6100);
}

#[tokio::test]
async fn observed_field_clipping_distinguishes_scroll_from_a_populated_field() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    page.evaluate(r#"document.body.innerHTML='<label>Short<input value="Ada" style="width:300px"></label><label>Long<input id="long" style="width:70px"></label>'; const field=document.querySelector('#long'); field.value='A long value whose beginning is outside the visible input'; field.focus(); field.setSelectionRange(field.value.length,field.value.length); field.scrollLeft=field.scrollWidth; true"#).await.unwrap();
    let observed = page.observe().await.unwrap();
    for label in ["Short", "Long"] {
        let action = observed["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["kind"] == "fill" && a["label"] == label)
            .unwrap();
        assert_eq!(action["value_clipped"] == true, label == "Long", "{action}");
    }
}
