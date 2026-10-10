// A hub of links and buttons, one per trap: each trap is named in the result,
// and a navigation ends the batch.

/// A tiny site: a hub with one link or button per trap.
struct Traps {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Traps {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Traps {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let log = log.clone();
                tokio::spawn(async move {
                    let Ok((path, _)) = support::read_request(&mut socket).await else {
                        return;
                    };
                    log.lock().unwrap().push(path.clone());
                    let (status, body) = match path.as_str() {
                        "/report" => (
                            "500 Internal Server Error",
                            "<title>Report failed</title><h1>Internal error</h1>",
                        ),
                        "/account" => (
                            "200 OK",
                            "<title>Sign in</title><h1>Sign in to continue</h1>\
                             <input type=password aria-label=Password>",
                        ),
                        "/help" => ("200 OK", "<title>Help centre</title><h1>Help</h1>"),
                        "/secret" => (
                            "200 OK",
                            "<title>Echo</title>\
                             <input id=p type=password aria-label=Password \
                              style='position:absolute;left:20px;top:20px;width:260px;height:40px'>\
                             <div id=echo style='position:absolute;left:20px;top:100px'></div>\
                             <script>p.oninput=()=>echo.textContent=p.value</script>",
                        ),
                        _ => (
                            "200 OK",
                            "<title>Trap hub</title>\
                             <style>a,button{position:absolute;left:20px;width:240px;height:40px;\
                              font:16px Arial;display:block}\
                              #a{top:20px}#b{top:80px}#c{top:140px}#d{top:200px}</style>\
                             <a id=a href='/report'>Weekly report</a>\
                             <a id=b href='/account'>My account</a>\
                             <a id=c href='/help' target=_blank>Help centre</a>\
                             <button id=d onclick=\"if(confirm('Delete every record?'))\
                              document.title='deleted'\">Delete all</button>",
                        ),
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: text/html\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
    fn seen(&self, path: &str) -> bool {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .any(|seen| seen == path)
    }
}

/// A browser-level tab, which is how a page that opens a tab is followed.
async fn browser_tab(browser: &support::Browser) -> DirectTab {
    let (_, target_id) = first_page(browser).await;
    DirectTab::Target {
        endpoint: browser.endpoint.clone(),
        target_id,
    }
}

async fn hub_session(tab: &DirectTab, traps: &Traps) -> DirectSession {
    let mut session = DirectSession::attach(tab, Arc::new(OpenGuard), false)
        .await
        .unwrap();
    let loaded = session.run("navigate", &json!({"url": traps.url})).await;
    assert!(!loaded.is_error, "{}", loaded.text);
    session
}

#[tokio::test]
async fn each_trap_is_named_in_the_result_text() {
    let Some(browser) = support::Browser::start().await.unwrap() else {
        return;
    };
    let traps = Traps::start().await;
    let tab = browser_tab(&browser).await;
    let mut named = Vec::new();
    for (label, y) in [
        ("report", 40),
        ("account", 100),
        ("help", 160),
        ("delete", 220),
    ] {
        let mut session = hub_session(&tab, &traps).await;
        let step = session
            .run_computer(&actions(json!([
                {"type":"click","button":"left","x":60,"y":y},
                {"type":"screenshot"},
            ])))
            .await;
        assert!(!step.is_error, "{label}: {}", step.text);
        assert!(step.image.is_some(), "{label} keeps its screenshot");
        let result = tool_result("id", "computer", &step);
        let notes = notes(&result.data);
        assert!(
            !notes.is_empty(),
            "{label}: no notes; text: {}",
            result.text
        );
        assert!(
            result.text.contains("untrusted") && result.text.contains(&notes[0]),
            "{label}: the notes are in the text the model reads: {}",
            result.text
        );
        assert!(
            notes.len() <= 8 && notes.iter().all(|note| note.chars().count() <= 160),
            "{label}: {notes:?}"
        );
        named.push((label, notes.join("\n")));
    }
    let said = |label: &str| named.iter().find(|(l, _)| *l == label).unwrap().1.clone();
    let server_error = said("report");
    assert!(
        server_error.contains("HTTP 500") && server_error.contains("/report"),
        "{server_error}"
    );
    let login = said("account");
    assert!(login.contains("sign-in"), "{login}");
    let new_tab = said("help");
    assert!(
        new_tab.contains("new tab") && new_tab.contains("/help"),
        "{new_tab}"
    );
    let confirm = said("delete");
    assert!(
        confirm.contains("confirm dialog")
            && confirm.contains("Delete every record?")
            && confirm.contains("dismissed"),
        "{confirm}"
    );
}

#[tokio::test]
async fn a_batch_stops_after_a_navigation_and_names_what_it_did_not_run() {
    let Some(browser) = support::Browser::start().await.unwrap() else {
        return;
    };
    let traps = Traps::start().await;
    let tab = browser_tab(&browser).await;
    let mut session = hub_session(&tab, &traps).await;
    let step = session
        .run_computer(&actions(json!([
            {"type":"click","button":"left","x":60,"y":40},
            {"type":"click","button":"left","x":60,"y":100},
            {"type":"type","text":"hunter2"},
            {"type":"keypress","keys":["ENTER"]},
        ])))
        .await;
    assert!(
        !step.is_error,
        "stopping early is not a failure: {}",
        step.text
    );
    assert_eq!(step.data["completed_actions"], 1);
    assert_eq!(step.data["requested_actions"], 4);
    assert_eq!(step.data["unrun_actions"], 3);
    assert_eq!(step.data["stopped_after"], "navigation");
    assert!(
        traps.seen("/report") && !traps.seen("/account"),
        "the second click must not run on the new page: {:?}",
        traps.requests.lock().unwrap()
    );
    let notes = notes(&step.data).join("\n");
    assert!(
        notes.contains("not run")
            && notes.contains("click (60,100)")
            && notes.contains("type")
            && notes.contains("Enter"),
        "{notes}"
    );
    assert!(
        !notes.contains("hunter2"),
        "an unrun action is named, not its text: {notes}"
    );
    assert!(step.image.is_some());
}

#[tokio::test]
async fn an_address_that_holds_a_remembered_secret_does_not_stop_a_batch_that_stayed_put() {
    let Some(browser) = support::Browser::start().await.unwrap() else {
        return;
    };
    let traps = Traps::start().await;
    let (websocket, target_id) = first_page(&browser).await;
    let tab = DirectTab::Page {
        websocket,
        target_id,
    };
    // The owner remembers a code typed earlier, and the page's address holds
    // it: the look hides it, the address read before an action does not.
    let guard = Remembering(Mutex::new(vec!["hunter22".to_string()]));
    let mut session = DirectSession::attach(&tab, Arc::new(guard), false)
        .await
        .unwrap();
    let loaded = session
        .run(
            "navigate",
            &json!({"url": format!("{}?code=hunter22", traps.url)}),
        )
        .await;
    assert!(!loaded.is_error, "{}", loaded.text);
    // A confirm dialog is declined: the page stays where it is both times.
    let step = session
        .run_computer(&actions(json!([
            {"type":"click","button":"left","x":60,"y":220},
            {"type":"click","button":"left","x":60,"y":220},
        ])))
        .await;
    assert!(!step.is_error, "{}", step.text);
    assert_eq!(
        step.data["completed_actions"], 2,
        "the same page is not a navigation: {}",
        step.data
    );
    assert!(step.data.get("stopped_after").is_none(), "{}", step.data);
    let notes = notes(&step.data).join("\n");
    assert!(
        notes.contains("confirm dialog") && !notes.contains("loaded"),
        "{notes}"
    );
    assert!(!notes.contains("hunter22"), "{notes}");
}
