// Editing chords on a Mac page and a page that reports another platform, and a
// batch with nothing to say.

/// Makes the page report `navigator.platform` as `platform` for as long as
/// this value lives: a DevTools override lasts as long as its connection.
struct Platform(tokio::task::JoinHandle<()>);
impl Drop for Platform {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn report_platform(websocket: &str, platform: &str) -> Platform {
    let (mut socket, _) = tokio_tungstenite::connect_async(websocket).await.unwrap();
    let mut ask = async |id: u64, method: &str, params: Value| -> Value {
        socket
            .send(Message::Text(
                json!({"id": id, "method": method, "params": params})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        loop {
            let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                continue;
            };
            let reply: Value = serde_json::from_str(&text).unwrap();
            if reply["id"] == id {
                assert!(reply.get("error").is_none(), "{method}: {reply}");
                return reply["result"].clone();
            }
        }
    };
    let agent = ask(
        1,
        "Runtime.evaluate",
        json!({"expression": "navigator.userAgent", "returnByValue": true}),
    )
    .await["result"]["value"]
        .clone();
    ask(
        2,
        "Emulation.setUserAgentOverride",
        json!({"userAgent": agent, "platform": platform}),
    )
    .await;
    Platform(tokio::spawn(async move {
        while socket.next().await.is_some() {}
    }))
}

/// The failing batch of the saved native run: select all with Control, type,
/// Shift+a, Enter. On a Mac page Control+a selects nothing, so the typed text
/// lands after the old text.
fn control_a_batch() -> ComputerActions {
    actions(json!([
        {"type":"click","button":"left","x":60,"y":40},
        {"type":"click","button":"left","x":60,"y":100},
        {"type":"type","text":"penguin 🐧"},
        {"type":"keypress","keys":["CTRL","a"]},
        {"type":"type","text":"orca"},
        {"type":"keypress","keys":["SHIFT","a"]},
        {"type":"keypress","keys":["ENTER"]},
    ]))
}

struct Ran {
    step: DirectStep,
    events: Vec<Value>,
}

/// Run the batch on the fixture page, which reports `platform`.
async fn run_control_a(platform: &str) -> Option<Ran> {
    let browser = support::Browser::start().await.unwrap()?;
    let fixture = support::Fixture::start().await.unwrap();
    let (websocket, target_id) = first_page(&browser).await;
    let tab = DirectTab::Page {
        websocket: websocket.clone(),
        target_id,
    };
    let mut session = DirectSession::attach(&tab, Arc::new(OpenGuard), false)
        .await
        .unwrap();
    let loaded = session.run("navigate", &json!({"url": fixture.url})).await;
    assert!(!loaded.is_error, "{}", loaded.text);
    // After the load: an override set before it does not reach the new document.
    let _platform = report_platform(&websocket, platform).await;
    let step = session.run_computer(&control_a_batch()).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let events = fixture.events.lock().await.clone();
    Some(Ran { step, events })
}

fn trusted_submits(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event["type"] == "submit" && event["trusted"] == true)
        .map(|event| event["value"].as_str().unwrap().to_string())
        .collect()
}

fn letter_a_keydown(events: &[Value]) -> Option<&Value> {
    events
        .iter()
        .find(|event| event["type"] == "key" && event["key"] == "a")
}

#[tokio::test]
async fn control_a_on_a_mac_page_selects_all_and_submits_once() {
    let Some(ran) = run_control_a("MacIntel").await else {
        return;
    };
    assert!(!ran.step.is_error, "{}", ran.step.text);
    assert_eq!(
        trusted_submits(&ran.events),
        ["orcaA"],
        "Control+a must replace the old text: {:?}",
        ran.events
    );
    let key = letter_a_keydown(&ran.events).expect("the page saw the select-all key");
    assert_eq!(
        (key["ctrl"].clone(), key["meta"].clone()),
        (json!(false), json!(true)),
        "Control was sent as Command"
    );
    let notes = notes(&ran.step.data);
    assert!(
        notes
            .iter()
            .any(|note| note.contains("Ctrl+A was sent as Cmd+A on macOS")),
        "{notes:?}"
    );
    assert!(
        ran.step.text.contains("Ctrl+A was sent as Cmd+A on macOS"),
        "{}",
        ran.step.text
    );
}

#[tokio::test]
async fn control_a_on_a_non_mac_page_is_sent_as_control() {
    let Some(ran) = run_control_a("Win32").await else {
        return;
    };
    let key = letter_a_keydown(&ran.events).expect("the page saw the select-all key");
    assert_eq!(
        (key["ctrl"].clone(), key["meta"].clone()),
        (json!(true), json!(false)),
        "a non-Mac page keeps Control"
    );
    assert!(
        !ran.step.text.contains("macOS") && notes(&ran.step.data).is_empty(),
        "no remap, so nothing to say: {:?}",
        ran.step.data["computer_notes"]
    );
}

#[tokio::test]
async fn an_uneventful_batch_adds_nothing_to_the_result() {
    let Some(browser) = support::Browser::start().await.unwrap() else {
        return;
    };
    let fixture = support::Fixture::start().await.unwrap();
    let (websocket, target_id) = first_page(&browser).await;
    let tab = DirectTab::Page {
        websocket,
        target_id,
    };
    let mut session = DirectSession::attach(&tab, Arc::new(OpenGuard), false)
        .await
        .unwrap();
    let loaded = session.run("navigate", &json!({"url": fixture.url})).await;
    assert!(!loaded.is_error, "{}", loaded.text);
    // A control pressed and a field filled: nothing a model must be told, so
    // the replayed prefix gets no new text for it.
    let step = session
        .run_computer(&actions(json!([
            {"type":"click","button":"left","x":60,"y":40},
            {"type":"click","button":"left","x":60,"y":100},
            {"type":"type","text":"orca"},
        ])))
        .await;
    assert!(!step.is_error, "{}", step.text);
    assert!(step.data.get("computer_notes").is_none(), "{}", step.data);
    assert!(
        step.text
            .starts_with("UNTRUSTED browser observation.\nScreenshot of the tab attached"),
        "{}",
        step.text
    );
    assert_eq!(step.data["completed_actions"], 3);
    assert!(step.data.get("stopped_after").is_none());
}
