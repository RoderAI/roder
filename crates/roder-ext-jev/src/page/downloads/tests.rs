//! Keeping saved files: what a read reports, repeats, and the text of a
//! file.

use super::*;

fn saved(name: &str, content: &str) -> DownloadEvent {
    let folder = std::env::temp_dir().join(format!("jev-download-test-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let guid = rand_suffix().to_string();
    let path = folder.join(&guid);
    std::fs::write(&path, content).unwrap();
    DownloadEvent {
        guid,
        url: format!("http://shop.test/files/{name}"),
        name: name.into(),
        frame: "T".into(),
        state: DownloadState::Completed,
        bytes: content.len() as u64,
        file_path: Some(path.display().to_string()),
    }
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[test]
fn a_saved_file_is_news_once_and_counts_in_the_fingerprint() {
    let mut downloads = Downloads::default();
    let mut page = json!({"text": "Orders\nExport CSV", "fingerprint": "f"});
    downloads.report(&mut page);
    assert_eq!(
        page,
        json!({"text": "Orders\nExport CSV", "fingerprint": "f"})
    );

    let first = saved("orders.csv", "id,total\n1003,42.50\n");
    downloads.keep(&first);
    downloads.report(&mut page);
    assert_eq!(page["fingerprint"], "f|downloads:1");
    let text = page["text"].as_str().unwrap();
    assert!(
        text.starts_with("[Downloaded file: orders.csv (20 bytes, saved) from http://shop.test/files/orders.csv, saved to "),
        "{text}"
    );
    assert!(
        text.contains("]\nid,total\n1003,42.50\n\nOrders\nExport CSV"),
        "{text}"
    );
    assert_eq!(page["downloads"][0]["name"], "orders.csv");
    assert_eq!(page["downloads"][0]["size_bytes"], 20);
    assert!(page["downloads"][0].get("repeated").is_none());

    // The same file again: no news in the fingerprint, and the copy goes.
    let again = saved("orders.csv", "id,total\n1003,42.50\n");
    let mut next = json!({"text": "Orders", "fingerprint": "f"});
    downloads.keep(&again);
    downloads.report(&mut next);
    assert_eq!(next["fingerprint"], "f|downloads:1");
    assert_eq!(next["downloads"][0]["repeated"], true);
    assert_eq!(next["downloads"][0]["path"], page["downloads"][0]["path"]);
    let copy = Path::new(again.file_path.as_deref().unwrap());
    assert!(!copy.exists());
    assert!(!copy.parent().unwrap().join("orders.csv").exists());
    let first = page["downloads"][0]["path"].as_str().unwrap();
    assert!(first.ends_with("/orders.csv"), "{first}");
    assert_eq!(
        std::fs::read_to_string(first).unwrap(),
        "id,total\n1003,42.50\n"
    );
    assert!(
        next["text"]
            .as_str()
            .unwrap()
            .starts_with("[Download: saved orders.csv again, the same file as before")
    );

    // New content under the same name is a new file.
    let changed = saved("orders.csv", "id,total\n1004,7.00\n");
    let mut last = json!({"text": "", "fingerprint": "f"});
    downloads.keep(&changed);
    downloads.report(&mut last);
    assert_eq!(last["fingerprint"], "f|downloads:2");
    // Reported once: the read after it has no news.
    let mut quiet = json!({"text": "Orders", "fingerprint": "f"});
    downloads.report(&mut quiet);
    assert!(quiet.get("downloads").is_none());
    assert_eq!(quiet["text"], "Orders");
}

#[test]
fn binary_files_have_no_text_and_long_text_is_cut() {
    assert_eq!(text_of(b"%PDF\0\x01"), None);
    assert_eq!(text_of(&[0xff, 0xfe, 0x41]), None);
    // A character cut by the read limit is dropped, not the text.
    assert_eq!(
        text_of("caf\u{e9}".as_bytes()[..4].as_ref()).as_deref(),
        Some("caf")
    );
    let long = "x".repeat(SHOWN_CHARS + 5);
    let download = JevDownload {
        name: "log.txt".into(),
        url: "blob:http://a.test/1".into(),
        path: None,
        size_bytes: long.len() as u64,
        incomplete: false,
        repeated: false,
    };
    let line = notice(&download, Some(&long));
    assert!(
        line.ends_with("[Download text cut: 5 more characters in the file]"),
        "{line}"
    );
    assert!(
        line.starts_with(
            "[Downloaded file: log.txt (2005 bytes, saved) from blob:http://a.test/1]\n"
        )
    );
}

/// A `data:` address holds the whole file; only its start is kept.
#[test]
fn a_long_address_is_cut() {
    let mut downloads = Downloads::default();
    let mut file = saved("notes.txt", "Ship by Friday.\n");
    file.url = format!("data:text/plain,{}", "x".repeat(5_000));
    downloads.keep(&file);
    let mut page = json!({"text": "Notes", "fingerprint": "f"});
    downloads.report(&mut page);
    let url = page["downloads"][0]["url"].as_str().unwrap();
    assert_eq!(url.chars().count(), URL_CHARS + 1);
    assert!(url.starts_with("data:text/plain,xxx") && url.ends_with('…'));
    assert!(
        page["text"]
            .as_str()
            .unwrap()
            .ends_with("Ship by Friday.\n\nNotes")
    );
}
