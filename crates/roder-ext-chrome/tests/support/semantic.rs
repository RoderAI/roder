// Real-browser regression: DOM grouping, stable actionable refs, private action
// probes, reconnects, URL resets and bounded snapshots all share one sequence.
async fn semantic_observations(registry: &ToolRegistry, url: &str) {
    call(registry, "chrome_navigate", json!({"url":url})).await;
    eval(registry, r#"document.body.innerHTML = `<main><h1>Flight search</h1>
      <form aria-label="Flights"><label>To <input id="to" role="combobox" aria-expanded="false"></label>
      <ul role="listbox" aria-label="Destinations" id="suggestions" hidden></ul>
      <button type="button" id="choose">Open suggestions</button></form>
      <p role="status" id="price">Total $770</p>
      <table aria-label="Flat Rate"><tr><th>Box</th><th>Inside</th></tr>
        <tr><td>Medium</td><td>11 x 8.5 x 5.5 inches</td></tr></table>
      <div style="height:3000px"></div><button>Distant control</button></main>`;
      document.querySelector('#choose').onclick = () => {
        document.querySelector('#to').setAttribute('aria-expanded','true');
        document.querySelector('#suggestions').hidden = false;
        document.querySelector('#suggestions').innerHTML = '<li role="option">London (all airports)</li>';
      }; true"#).await;
    let full = call(registry, "chrome_page_snapshot", json!({})).await;
    assert!(full.text.contains("Snapshot (act by ref"), "{}", full.text);
    assert!(full.text.contains("form \"Flights\""), "{}", full.text);
    assert!(full.text.contains("heading \"Flight search\""), "{}", full.text);
    assert!(full.text.contains("table \"Flat Rate\""), "{}", full.text);
    assert!(full.text.contains("cell \"11 x 8.5 x 5.5 inches\""), "{}", full.text);
    assert!(!full.data["page"]["snapshot"]["rows"].as_array().unwrap().iter()
        .any(|row| row["label"] == "Distant control"));
    let page = &full.data["page"];
    let controls = page["elements"].as_array().unwrap();
    let choose = controls.iter().find(|e| e["label"] == "Open suggestions").unwrap()["ref"].clone();
    let to = controls.iter().find(|e| e["label"] == "To").unwrap()["ref"].clone();
    // chrome_* reconnects for every tool. An internal before-action probe runs
    // during click: neither should lose the published full snapshot baseline.
    let clicked = call(registry, "chrome_click", json!({"ref":choose})).await;
    assert!(!clicked.is_error, "{}", clicked.text);
    assert_eq!(clicked.data["page"]["snapshot"]["mode"], "diff");
    assert!(clicked.text.contains("[expanded]"), "{}", clicked.text);
    assert!(clicked.text.contains("London (all airports)"), "{}", clicked.text);
    assert!(!clicked.text.contains("heading \"Flight search\""), "unchanged headings should not repeat: {}", clicked.text);
    let rows = clicked.data["page"]["snapshot"]["rows"].as_array().unwrap();
    let option = rows.iter().find(|row| row["role"] == "option").unwrap()["ref"].clone();
    assert!(option.is_string(), "options must carry real actionable refs");
    let typed = call(registry, "chrome_type", json!({"ref":to,"text":"London"})).await;
    assert!(!typed.is_error, "{}", typed.text);
    assert!(typed.text.contains("= \"London\""), "{}", typed.text);
    eval(registry, "document.querySelector('#suggestions').innerHTML = ''; true").await;
    let waited = call(registry, "chrome_keypress", json!({"key":"Escape"})).await;
    assert!(waited.text.contains(&format!("- {} option", option.as_str().unwrap())), "{}", waited.text);
    assert!(waited.text.contains("London (all airports)"));
    // Hash navigation is a new visible page, even when its DOM survives.
    eval(registry, "location.hash = 'new'; true").await;
    let navigated = call(registry, "chrome_keypress", json!({"key":"Escape"})).await;
    assert_eq!(navigated.data["page"]["snapshot"]["mode"], "full");
    let looked = call(registry, "chrome_page_snapshot", json!({})).await;
    assert_eq!(looked.data["page"]["snapshot"]["mode"], "full");
    assert!(looked.text.contains("= \"London\""));
    eval(registry, "for(let i=0;i<180;i++){const b=document.createElement('button');b.style.cssText='position:fixed;left:0;top:0';b.textContent='Extra '+i;document.body.append(b)}; true").await;
    let limited = call(registry, "chrome_keypress", json!({"key":"Escape"})).await;
    assert_eq!(limited.data["page"]["snapshot"]["mode"], "full");
    assert_eq!(limited.data["page"]["snapshot"]["limited"], true);
    assert_eq!(limited.data["page"]["snapshot"]["removed"], json!([]));
    assert!(limited.text.contains("unlisted nodes are not evidence of removal"));
    // Optional evidence for the paired formatter eval; no network or model.
    if let Some(dir) = std::env::var_os("RODER_SEMANTIC_EVAL_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let evidence = json!([
            {"step":"full", "page":full.data["page"], "text":full.text},
            {"step":"open suggestions", "page":clicked.data["page"], "text":clicked.text},
            {"step":"type London", "page":typed.data["page"], "text":typed.text},
            {"step":"remove suggestions", "page":waited.data["page"], "text":waited.text},
            {"step":"new URL", "page":navigated.data["page"], "text":navigated.text},
            {"step":"full recovery", "page":looked.data["page"], "text":looked.text},
            {"step":"limited snapshot", "page":limited.data["page"], "text":limited.text}
        ]);
        std::fs::write(dir.join("semantic-observations.json"), serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
}
