// Secrets echoed into ordinary text must be scrubbed before display truncation.
async fn semantic_secrets(registry: &ToolRegistry, browser: &Browser, url: &str) {
    use roder_ext_chrome::direct::DirectGuard;
    struct Hides(String);
    impl DirectGuard for Hides {
        fn scrub(&self, text: &str) -> String {
            text.replace(&self.0, "[secret]")
        }
    }
    call(registry, "chrome_navigate", json!({"url":url})).await;
    let secret = "boundary-sensitive-secret-".repeat(5);
    let script = format!(r#"document.body.innerHTML='<main><h1></h1><button></button><input></main>';const s={};document.querySelector('h1').textContent=s;document.querySelector('button').textContent=s;document.querySelector('input').value=s;document.querySelector('main').append(document.createTextNode('z'.repeat(2950)+s));true"#, serde_json::to_string(&secret).unwrap());
    eval(registry, &script).await;
    let tabs = call(registry, "chrome_tabs_list", json!({})).await;
    let target = tabs.data["tabs"].as_array().unwrap().iter().find(|t| t["url"] == url).unwrap()["targetId"].as_str().unwrap();
    let tab = DirectTab::Target { endpoint: browser.endpoint.clone(), target_id: target.into() };
    let mut session = DirectSession::attach(&tab, Arc::new(Hides(secret.clone())), false).await.unwrap();
    for operation in ["look", "key"] {
        let args = if operation == "key" { json!({"key":"Escape"}) } else { json!({}) };
        let result = session.run(operation, &args).await;
        assert!(!result.is_error, "{}", result.text);
        let public = result.data.to_string();
        assert!(!public.contains("boundary-sensitive"), "a clipped secret escaped in {operation}");
        assert!(!result.text.contains("boundary-sensitive"));
        if operation == "look" {
            assert!(result.text.contains("[secret]"), "{}", result.text);
        }
    }
}
