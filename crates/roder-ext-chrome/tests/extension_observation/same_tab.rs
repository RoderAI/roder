// Page commands are answered one at a time. Two clicks at once, as a turn's
// parallel tool calls make them, are sent one after the other, and each is
// compared with the page the click before it left: not with whichever answer
// came back first, which would say nothing changed for the click that did it.

/// An extension that answers a `page/click` after a pause (the first the
/// longest) and counts how many commands it holds at once.
struct Paused {
    in_flight: std::sync::atomic::AtomicUsize,
    most_in_flight: std::sync::atomic::AtomicUsize,
    clicks: std::sync::atomic::AtomicUsize,
    after_click: Value,
    seed: Value,
}

#[async_trait::async_trait]
impl ChromeController for Paused {
    fn status(&self) -> ChromeStatus {
        ChromeStatus {
            connected: true,
            client_count: 1,
            enabled: true,
            mode: ChromePermissionMode::Control,
            ..ChromeStatus::default()
        }
    }
    fn set_enabled(&self, _: bool) {}
    fn set_mode(&self, _: ChromePermissionMode) {}
    async fn dispatch(&self, command: ChromeCommand) -> Result<Value, ChromeError> {
        use std::sync::atomic::Ordering::SeqCst;
        if command.kind == "page/snapshot" {
            return Ok(self.seed.clone());
        }
        let nth = self.clicks.fetch_add(1, SeqCst);
        let now = self.in_flight.fetch_add(1, SeqCst) + 1;
        self.most_in_flight.fetch_max(now, SeqCst);
        let pause = if nth == 0 { 150 } else { 10 };
        tokio::time::sleep(Duration::from_millis(pause)).await;
        self.in_flight.fetch_sub(1, SeqCst);
        Ok(self.after_click.clone())
    }
}

#[tokio::test]
async fn two_clicks_at_once_are_sent_in_turn_and_each_compared_with_the_one_before() {
    let before = snapshot("https://app.test/", "App", "zero", vec![]);
    let after = snapshot("https://app.test/", "App", "one", vec![]);
    let extension = Arc::new(Paused {
        in_flight: Default::default(),
        most_in_flight: Default::default(),
        clicks: Default::default(),
        after_click: observed_reply(json!({"ref": "c1"}), &after, 1),
        seed: snapshot_reply(&before),
    });
    let mut registry = ToolRegistry::default();
    ChromeToolContributor::with_controller(extension.clone())
        .contribute(&mut registry)
        .unwrap();
    let seeded = call(&registry, "chrome_page_snapshot", json!({})).await;
    assert!(!seeded.is_error, "{}", seeded.text);

    let (first, second) = tokio::join!(
        call(&registry, "chrome_click", json!({"ref": "c1"})),
        call(&registry, "chrome_click", json!({"ref": "c1"})),
    );

    assert_eq!(
        extension
            .most_in_flight
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the second click was sent before the first was answered"
    );
    // The first click is the one that changed the page; the second finds it
    // as the first left it. Out of order, the second to answer would be told
    // the text changed and the first that nothing did.
    assert_eq!(outcome(&first.text), "Page text changed.", "{}", first.text);
    assert_eq!(
        outcome(&second.text),
        "No visible change.",
        "{}",
        second.text
    );
}
