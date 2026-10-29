// A passive paragraph after scrolling must be readable without a screenshot.
async fn viewport_text(registry: &ToolRegistry, browser: &Browser, url: &str) {
    call(registry, "chrome_navigate", json!({"url":url})).await;
    eval(registry, r#"document.body.innerHTML=`<nav>Top navigation</nav>
      <div style="height:1500px"></div><p id="detail">Inside dimensions 11 by 8.5 by 5.5 inches. Retail price 25.80.</p>
      <div style="height:100px;overflow:hidden"><p>Visible panel text</p><div style="height:300px"></div><p>Clipped panel secret</p></div>
      <textarea>Private field value</textarea><p style="opacity:0">Transparent content</p><div style="height:1500px"></div>`;scrollTo(0,0);true"#).await;
    let before = call(registry, "chrome_page_snapshot", json!({})).await;
    assert!(!before.data["page"]["text"].as_str().unwrap().contains("Inside dimensions"));
    let tabs=call(registry,"chrome_tabs_list",json!({})).await;
    let target=tabs.data["tabs"].as_array().unwrap().iter().find(|t|t["url"]==url).unwrap()["targetId"].as_str().unwrap();
    let tab=DirectTab::Target {endpoint:browser.endpoint.clone(),target_id:target.into()};
    let mut direct=DirectSession::attach(&tab,Arc::new(OpenGuard),false).await.unwrap();
    let document=direct.run("look",&json!({"text_scope":"document"})).await;
    assert!(!document.is_error,"{}",document.text);
    assert!(document.text.contains("Inside dimensions"),"{}",document.text);
    assert!(!document.text.contains("Private field value"));
    assert!(!document.text.contains("Transparent content"));
    assert_eq!(eval(registry,"scrollY").await,json!(0));
    eval(registry, "document.querySelector('#detail').scrollIntoView({block:'start'});true").await;
    let after = call(registry, "chrome_keypress", json!({"key":"Escape"})).await;
    let text = after.data["page"]["text"].as_str().unwrap();
    assert!(text.contains("Inside dimensions"), "{text}");
    assert!(text.contains("Visible panel text"), "{text}");
    for hidden in ["Top navigation", "Clipped panel secret", "Private field value", "Transparent content"] {
        assert!(!text.contains(hidden), "{hidden}: {text}");
    }
    assert!(after.text.contains("Inside dimensions"), "{}", after.text);
    assert!(!after.text.contains("Page text unchanged"), "{}", after.text);
    eval(registry, "document.body.append(Object.assign(document.createElement('p'),{textContent:'界'.repeat(9000)}));true").await;
    let first=direct.run("look",&json!({"text_scope":"document"})).await;
    let offset=first.data["page"]["next_text_offset"].as_u64().expect("long text must be paged");
    assert_eq!(offset as usize,first.data["page"]["text"].as_str().unwrap().chars().count());
    assert!(first.text.lines().count()<110);
    let second=direct.run("look",&json!({"text_scope":"document","text_offset":offset})).await;
    assert_eq!(second.data["page"]["text_offset"],json!(offset));
    assert!(second.text.contains('界'));
    assert!(second.text.chars().count()<6600);

}
