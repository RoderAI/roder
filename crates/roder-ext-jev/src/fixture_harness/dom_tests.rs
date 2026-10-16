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
        "value=\"Ada\"",
        "combobox \"Country\"",
        "value=\"Chile\"",
        "button \"Save\" [disabled]",
        "expanded=false",
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
    assert!(dom.contains(&format!("node={}", more["node"])));
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
            .contains("DOM snapshot truncated")
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
