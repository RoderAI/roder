use super::*;

#[test]
fn a_configured_endpoint_may_be_http_or_a_websocket() {
    for url in [
        "http://127.0.0.1:9333",
        "https://chrome.example.com/devtools",
        "ws://127.0.0.1:9333/devtools/browser/abc",
        "wss://browser.example.com/session?token=secret",
    ] {
        assert_eq!(configured_url(&format!(" {url} ")).unwrap(), url);
    }
    for bad in [
        "127.0.0.1:9222",
        "file:///tmp/chrome",
        "ftp://example.com",
        "ws://",
        "http://",
        "not a url",
    ] {
        let error = configured_url(bad).unwrap_err().to_string();
        assert!(error.starts_with("JEV_CDP_URL must be"), "{bad}: {error}");
    }
    // The value itself never reaches the message: it may carry a token.
    let error = configured_url("gopher://example.com/?token=secret")
        .unwrap_err()
        .to_string();
    assert!(!error.contains("secret"), "{error}");
}

#[test]
fn only_a_loopback_endpoint_is_reported_in_full() {
    let reported = |url: &str| ChromeEndpoint::new(url, false).reported_url();
    for local in [
        "http://127.0.0.1:9222",
        "ws://127.0.0.1:9222/devtools/browser/abc",
        "http://localhost:9222",
        "ws://[::1]:9222/devtools/browser/abc",
    ] {
        assert_eq!(reported(local), local);
    }
    assert_eq!(
        reported("wss://browser.example.com:443/session?token=secret"),
        "wss://browser.example.com"
    );
    assert_eq!(
        reported("https://chrome.example.com:9222/t/secret"),
        "https://chrome.example.com"
    );
    // Credentials are never shown, even on loopback.
    assert_eq!(
        reported("http://user:secret@127.0.0.1:9222"),
        "http://127.0.0.1"
    );
}

#[test]
fn cdp_port_defaults_and_validates() {
    assert_eq!(cdp_port(None), DEFAULT_CDP_PORT);
    assert_eq!(cdp_port(Some(" 9333 ")), 9333);
    assert_eq!(cdp_port(Some("0")), DEFAULT_CDP_PORT);
    assert_eq!(cdp_port(Some("not-a-port")), DEFAULT_CDP_PORT);
}

#[test]
fn autostart_is_on_unless_explicitly_disabled() {
    assert!(autostart_enabled(None));
    assert!(autostart_enabled(Some("1")));
    assert!(!autostart_enabled(Some("0")));
    assert!(!autostart_enabled(Some(" Off ")));
    assert!(!autostart_enabled(Some("false")));
}

#[test]
fn explicit_binary_replaces_platform_candidates() {
    assert_eq!(
        chrome_candidates(Some("/opt/chrome/chrome")),
        vec!["/opt/chrome/chrome".to_string()]
    );
    assert!(chrome_candidates(Some("   ")).len() > 1);
    assert!(!chrome_candidates(None).is_empty());
}

#[test]
fn a_new_profile_gets_the_password_manager_turned_off() {
    let profile = std::env::temp_dir().join(format!("roder-jev-prefs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&profile);
    std::fs::create_dir_all(&profile).unwrap();
    quiet_password_manager(&profile).unwrap();
    let written: Value = serde_json::from_str(
        &std::fs::read_to_string(profile.join("Default/Preferences")).unwrap(),
    )
    .unwrap();
    assert_eq!(written["credentials_enable_service"], json!(false));
    assert_eq!(written["profile"]["password_manager_enabled"], json!(false));
    assert_eq!(
        written["profile"]["password_manager_leak_detection"],
        json!(false)
    );
    std::fs::remove_dir_all(&profile).ok();
}

#[test]
fn the_active_port_file_is_read_only_when_whole() {
    let file = std::env::temp_dir().join(format!("roder-jev-port-{}", std::process::id()));
    std::fs::write(&file, "53817\n/devtools/browser/abc\n").unwrap();
    assert_eq!(read_active_port(&file), Some(53817));
    std::fs::write(&file, "538").unwrap();
    assert_eq!(read_active_port(&file), Some(538));
    std::fs::write(&file, "").unwrap();
    assert_eq!(read_active_port(&file), None);
    std::fs::write(&file, "0\n").unwrap();
    assert_eq!(read_active_port(&file), None);
    std::fs::remove_file(&file).ok();
    assert_eq!(read_active_port(&file), None);
}

#[test]
fn endpoint_reports_launch_state() {
    let reused = ChromeEndpoint::new("http://127.0.0.1:9222", false);
    assert_eq!(reused.url(), "http://127.0.0.1:9222");
    assert!(!reused.launched());
    assert!(ChromeEndpoint::new("http://127.0.0.1:9222", true).launched());
}
