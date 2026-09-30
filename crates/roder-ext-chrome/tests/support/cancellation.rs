include!("cdp_relay.rs");

async fn cancellation_faults(tab: &DirectTab, registry: &ToolRegistry) {
    eval(registry, "window.heldKeys=[]; document.addEventListener('keydown',e=>heldKeys.push(e.key)); document.addEventListener('keyup',e=>{heldKeys=heldKeys.filter(k=>k!==e.key); window.releaseModifiers=[e.ctrlKey,e.altKey,e.metaKey,e.shiftKey]}); true").await;
    for (fault, name, args, restored) in [
        (
            Fault::Key,
            "key",
            json!({"key":"Control+ArrowDown"}),
            "heldKeys.length===0 && releaseModifiers.every(m=>!m)",
        ),
        (
            Fault::Screenshot,
            "screenshot",
            json!({}),
            "![...document.documentElement.children].some(e=>e.style.zIndex==='2147483647')",
        ),
        (
            Fault::Drag,
            "drag",
            json!({"from_x":0,"from_y":0,"to_x":100,"to_y":100}),
            "window.dragHeld===false",
        ),
    ] {
        let (routed, applied, relay) = relay(tab, fault).await;
        let mut session = DirectSession::attach(&routed, Arc::new(OpenGuard), false)
            .await
            .unwrap();
        if matches!(fault, Fault::Key) {
            // Cancel only the borrowed run future; its owner keeps the session.
            {
                let running = session.run(name, &args);
                tokio::pin!(running);
                tokio::select! {
                    _ = &mut running => panic!("keydown reply was not delayed"),
                    result = tokio::time::timeout(Duration::from_secs(5), applied) => result.unwrap().unwrap(),
                }
            }
            // A new operation must await recovery before driving this tab again.
            let resumed = session.run("look", &json!({})).await;
            assert!(!resumed.is_error, "{}", resumed.text);
            assert_eq!(eval(registry, restored).await, true);
            relay.abort();
            continue;
        }
        let task = tokio::spawn(async move {
            let step = session.run(name, &args).await;
            // Keep the session alive: error cleanup must happen before drop.
            (step, session)
        });
        tokio::time::timeout(Duration::from_secs(5), applied)
            .await
            .unwrap()
            .unwrap();
        match fault {
            Fault::Drag => {
                let (step, _session) = task.await.unwrap();
                assert!(step.is_error && step.text.contains("fixture drag failure"));
                assert_eq!(eval(registry, restored).await, true);
            }
            _ => {
                assert!(
                    !task.is_finished(),
                    "delayed reply did not hold the operation"
                );
                task.abort();
                assert!(matches!(task.await, Err(error) if error.is_cancelled()));
                let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
                while eval(registry, restored).await != true {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "cancelled {name} left browser state behind"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
        relay.abort();
    }
}
