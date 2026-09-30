// Real browser tab identity and optional Desktop scope enforcement.
async fn desktop_scope_and_tab_identity(browser: &Browser, registry: &ToolRegistry, url: &str) {
    let before = call(registry, "chrome_tabs_list", json!({})).await;
    let id = before.data["tabs"][0]["id"].clone();
    let target = before.data["tabs"][0]["targetId"].clone();
    assert!(
        before.text.contains(&format!("\"id\":{id}")),
        "model must receive the same numeric tab id as data"
    );
    let version = roder_ext_chrome::direct::devtools::browser_websocket(&browser.endpoint)
        .await
        .unwrap();
    let (mut socket, _) = tokio_tungstenite::connect_async(version).await.unwrap();
    socket
        .send(Message::Text(
            json!({"id":1,"method":"Target.createTarget","params":{"url":"about:blank"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    while let Some(Ok(message)) = socket.next().await {
        if let Message::Text(text) = message {
            let result: Value = serde_json::from_str(&text).unwrap();
            if result["id"] == 1 {
                assert!(result.get("error").is_none());
                break;
            }
        }
    }
    let after = call(registry, "chrome_tabs_list", json!({})).await;
    assert_eq!(
        after.data["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["targetId"] == target)
            .unwrap()["id"],
        id
    );
    let still_bound = call(registry, "chrome_page_snapshot", json!({})).await;
    assert!(
        still_bound.text.contains("Primitive fixture"),
        "new targets must not change the thread binding: {}",
        still_bound.text
    );
    let explicit = call(registry, "chrome_page_snapshot", json!({"tabId":id})).await;
    assert!(explicit.text.contains("Primitive fixture"));
    let unavailable = call(
        registry,
        "chrome_click",
        json!({"tabId":999999,"selector":"#inc"}),
    )
    .await;
    assert!(unavailable.is_error);
    unsafe {
        std::env::set_var("RODER_DESKTOP_ALLOWED_ORIGINS", url.trim_end_matches('/'));
    }
    let rejected = call(
        registry,
        "chrome_navigate",
        json!({"url":url.replace("127.0.0.1","localhost")}),
    )
    .await;
    assert!(rejected.is_error);
    assert_eq!(
        eval(registry, "document.querySelector('#count').textContent").await,
        "Count: 0"
    );
    let redirected = call(
        registry,
        "chrome_navigate",
        json!({"url":format!("{url}redirect")}),
    )
    .await;
    assert!(
        redirected.is_error,
        "redirect outside scope was not reported: {}",
        redirected.text
    );
    let reset = call(registry, "chrome_navigate", json!({"url":url})).await;
    assert!(!reset.is_error, "{}", reset.text);
    // An external navigation must be checked again before the next action.
    eval(
        registry,
        &format!(
            "location.href={}; true",
            json!(url.replace("127.0.0.1", "localhost"))
        ),
    )
    .await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let looked = call(registry, "chrome_page_snapshot", json!({})).await;
        if looked.is_error {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    for (name, args) in [
        ("chrome_click", json!({"selector":"#inc"})),
        ("chrome_screenshot", json!({})),
        (
            "chrome_eval",
            json!({"expression":"document.querySelector('#inc').click()"}),
        ),
    ] {
        let denied = call(registry, name, args).await;
        assert!(
            denied.is_error,
            "off-origin {name} succeeded: {}",
            denied.text
        );
    }
    let recovered = call(registry, "chrome_navigate", json!({"url":url})).await;
    assert!(!recovered.is_error, "{}", recovered.text);
    unsafe {
        std::env::remove_var("RODER_DESKTOP_ALLOWED_ORIGINS");
    }
}
