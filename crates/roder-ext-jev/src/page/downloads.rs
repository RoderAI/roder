//! Files an action downloads.
//!
//! A link whose target is a file, or a button whose script saves one, leaves
//! the page as it was: Chrome turns the navigation into a download and the
//! tab keeps its document. Read as a page, such a click did nothing, so the
//! model clicked the export again until the stall rule stopped the run.
//! Here every tab lets the browser save downloads into Jev's folder
//! (`<temp>/jev-downloads`, each finished file then moved to
//! `kept/<guid>/<name>` there) and announce them, the settle after an input
//! waits for the files that input started, and the next observation:
//! Chrome may announce a file after the page settled, so a navigation the
//! input started that has not resolved, or a save the page attempted
//! (`saves.js` counts clicks on download and blob links), gets up to 1.5 s
//! for its announcement; a click that did neither pays one evaluate.
//!
//! - says what was saved, where, and the start of its text, before the page
//!   text (`downloads` on the observation lists them);
//! - counts each distinct file in its fingerprint, so the step that saved it
//!   changed something. A file saved again with the same name and content
//!   does not count: an export clicked four times changed nothing after the
//!   first.
//!
//! Chrome keeps that setting only while the connection that set it is
//! open, and puts its own download behaviour back when Jev disconnects. The
//! rule follows fastbrowse's browser (MIT, `page/act.rs download_landed` and
//! `session.rs capture_download`), which reads the file off a paused
//! response instead.

use std::hash::Hasher;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::{Page, TEXT_CHARS};
use crate::cdp::{DownloadEvent, DownloadState, NavigationMark};
use crate::engine::JevDownload;

/// Jev's folder under the system's temporary directory.
const FOLDER: &str = "jev-downloads";
/// Where finished files go inside it, each in a folder of its guid.
const KEPT: &str = "kept";
/// How long a navigation the input started, still unresolved once the page
/// settled, may take to be announced as a download.
const ANNOUNCE_WAIT: Duration = Duration::from_millis(1500);
/// How long an announced download may take to be saved.
const SAVE_WAIT: Duration = Duration::from_secs(10);
/// How much of a file is read for its text.
const TEXT_BYTES: usize = 16_384;
/// How much of that text an observation shows.
const SHOWN_CHARS: usize = 2_000;
/// How much of a file's address is kept: a `data:` address holds the
/// whole file.
const URL_CHARS: usize = 300;

/// Counts the page's own attempts to save a file (see the script).
const SAVES_JS: &str = include_str!("../assets/saves.js");
/// Reads that count, and starts it again.
const TAKE_SAVES_JS: &str =
    "(() => { const n = window.__jevSaves | 0; window.__jevSaves = 0; return n; })()";

/// Where downloads are saved.
fn download_folder() -> PathBuf {
    std::env::temp_dir().join(FOLDER)
}

/// The files the run has saved, and those the next read reports.
#[derive(Default)]
pub(super) struct Downloads {
    /// (name, content hash) of every distinct file, with where it is.
    kept: Vec<(String, u64, Option<String>)>,
    /// Saved since the last read, with the start of their text.
    pending: Vec<(JevDownload, Option<String>)>,
}

impl Downloads {
    /// How many distinct files the run has saved.
    fn distinct(&self) -> usize {
        self.kept.len()
    }

    /// Keep one saved file: read, hashed and compared with those before it.
    fn keep(&mut self, event: &DownloadEvent) {
        let incomplete = event.state == DownloadState::InProgress;
        // Chrome saves it under its guid, and a finished one moves to
        // `kept/<guid>/<name>`: two files of one name never overwrite each
        // other.
        let saved = event
            .file_path
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| download_folder().join(&event.guid));
        let path = match incomplete {
            true => saved.exists().then_some(saved),
            false => place(&saved, &event.guid, &event.name),
        };
        let read = path.as_deref().and_then(read_file);
        let incomplete = event.state == DownloadState::InProgress;
        let (size, hash, text) = match read {
            Some(read) if !incomplete => (read.size, Some(read.hash), read.text),
            _ => (event.bytes, None, None),
        };
        let earlier = hash.and_then(|hash| {
            self.kept
                .iter()
                .find(|(name, kept, _)| *name == event.name && *kept == hash)
        });
        let repeated = earlier.is_some();
        let mut shown = path.as_deref().map(|path| path.display().to_string());
        if let Some((_, _, first)) = earlier {
            // The copy says nothing the first did not.
            if let Some(copy) = &path
                && first.as_deref() != shown.as_deref()
            {
                std::fs::remove_file(copy).ok();
                if let Some(folder) = copy.parent().filter(|folder| folder.ends_with(&event.guid)) {
                    std::fs::remove_dir(folder).ok();
                }
            }
            shown = first.clone();
        } else {
            self.kept
                .push((event.name.clone(), hash.unwrap_or_default(), shown.clone()));
        }
        let download = JevDownload {
            name: event.name.clone(),
            url: short_url(&event.url),
            path: shown,
            size_bytes: size,
            incomplete,
            repeated,
        };
        self.pending
            .push((download, (!repeated).then_some(text).flatten()));
    }

    /// Put what was saved since the last read on `observation`: a line each
    /// (and the start of a readable file's text) before the page text, and
    /// `downloads`. The fingerprint counts every distinct file so far.
    pub(super) fn report(&mut self, observation: &mut Value) {
        if self.distinct() > 0
            && let Some(fingerprint) = observation["fingerprint"].as_str()
        {
            observation["fingerprint"] =
                json!(format!("{fingerprint}|downloads:{}", self.distinct()));
        }
        let pending = std::mem::take(&mut self.pending);
        if pending.is_empty() {
            return;
        }
        let mut lines = pending
            .iter()
            .map(|(download, text)| notice(download, text.as_deref()))
            .collect::<Vec<_>>();
        if let Some(text) = observation["text"].as_str().filter(|text| !text.is_empty()) {
            lines.push(text.to_string());
        }
        let text = lines
            .join("\n")
            .chars()
            .take(TEXT_CHARS)
            .collect::<String>();
        observation["text"] = json!(text);
        observation["downloads"] = json!(
            pending
                .into_iter()
                .map(|(download, _)| download)
                .collect::<Vec<_>>()
        );
    }
}

/// Move a finished file Chrome saved at `saved` to `kept/<guid>/<name>`
/// beside it; where it is now, or `None` when there is no file.
fn place(saved: &Path, guid: &str, name: &str) -> Option<PathBuf> {
    if !saved.is_file() {
        return None;
    }
    let name = safe_name(name);
    let folder = saved.parent()?.join(KEPT).join(safe_name(guid));
    let target = folder.join(name);
    if std::fs::create_dir_all(&folder).is_ok() && std::fs::rename(saved, &target).is_ok() {
        return Some(target);
    }
    Some(saved.to_path_buf())
}

/// `url`, cut to [`URL_CHARS`] with the cut marked.
fn short_url(url: &str) -> String {
    match url.char_indices().nth(URL_CHARS) {
        Some((end, _)) => format!("{}…", &url[..end]),
        None => url.to_string(),
    }
}

/// A name that stays one path segment.
fn safe_name(name: &str) -> String {
    let name = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            c => c,
        })
        .collect::<String>();
    match name.trim_start_matches('.') {
        "" => "download".to_string(),
        _ => name,
    }
}

/// The line an observation shows for one saved file.
fn notice(download: &JevDownload, text: Option<&str>) -> String {
    let name = &download.name;
    if download.repeated {
        return format!(
            "[Download: saved {name} again, the same file as before; nothing new was downloaded]"
        );
    }
    let place = download
        .path
        .as_deref()
        .map(|path| format!(", saved to {path}"))
        .unwrap_or_default();
    let state = match download.incomplete {
        true => "still saving",
        false => "saved",
    };
    let mut line = format!(
        "[Downloaded file: {name} ({} bytes, {state}) from {}{place}]",
        download.size_bytes, download.url
    );
    if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
        let total = text.chars().count();
        line.push('\n');
        line.extend(text.chars().take(SHOWN_CHARS));
        if total > SHOWN_CHARS {
            line.push_str(&format!(
                "\n[Download text cut: {} more characters in the file]",
                total - SHOWN_CHARS
            ));
        }
    }
    line
}

struct ReadFile {
    size: u64,
    hash: u64,
    text: Option<String>,
}

/// A saved file's size, a hash of its content, and its text when its start
/// reads as text (UTF-8 without NUL bytes).
fn read_file(path: &Path) -> Option<ReadFile> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut head = Vec::new();
    let mut size = 0u64;
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        hasher.write(&chunk[..read]);
        size += read as u64;
        let room = TEXT_BYTES.saturating_sub(head.len());
        head.extend_from_slice(&chunk[..read.min(room)]);
    }
    Some(ReadFile {
        size,
        hash: hasher.finish(),
        text: text_of(&head),
    })
}

fn text_of(head: &[u8]) -> Option<String> {
    if head.contains(&0) {
        return None;
    }
    let valid = match std::str::from_utf8(head) {
        Ok(text) => text,
        // The read may have cut a character in two; anything else is binary.
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&head[..error.valid_up_to()]).ok()?
        }
        Err(_) => return None,
    };
    Some(valid.to_string())
}

impl Page {
    /// Have the browser save the downloads of this tab's browser context
    /// into Jev's folder and announce them, and count the page's own
    /// attempts to save a file on every document. Best effort: a browser
    /// that refuses leaves downloads to itself, and they read as before.
    pub(super) async fn allow_downloads(&mut self) {
        self.call(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": SAVES_JS}),
        )
        .await
        .ok();
        // The document shown already (an adopted or resumed tab).
        self.evaluate(SAVES_JS).await.ok();
        let folder = download_folder();
        if std::fs::create_dir_all(&folder).is_err() {
            return;
        }
        let target = self.tab().target.clone();
        let mut params = json!({
            "behavior": "allowAndName",
            "downloadPath": folder.display().to_string(),
            "eventsEnabled": true,
        });
        if let Ok(info) = self
            .connection
            .call("Target.getTargetInfo", json!({"targetId": target}), None)
            .await
            && let Some(context) = info["targetInfo"]["browserContextId"].as_str()
        {
            params["browserContextId"] = json!(context);
        }
        self.connection
            .call("Browser.setDownloadBehavior", params, None)
            .await
            .ok();
    }

    /// After an input settled: wait for the files it started downloading
    /// and keep each for the next read. A navigation the input started that
    /// the page has not resolved, or a save the page attempted (a click on
    /// a download link, a script's `saveAs`), may be a file on its way, and
    /// gets a short wait to be announced; an announced file gets its time to
    /// be saved.
    pub(super) async fn await_downloads(&mut self, mark: Option<NavigationMark>) {
        let Some(mark) = mark else {
            return;
        };
        let saving = self
            .evaluate(TAKE_SAVES_JS)
            .await
            .ok()
            .and_then(|saves| saves.as_u64())
            .is_some_and(|saves| saves > 0);
        let session = self.tab().session.clone();
        let frames = self
            .tabs
            .iter()
            .map(|tab| tab.target.clone())
            .chain(self.stray.iter().cloned())
            .collect::<Vec<_>>();
        let ours = move |download: &DownloadEvent| frames.contains(&download.frame);
        let announced = |connection: &crate::cdp::Connection| {
            connection.downloads_since(mark).iter().any(&ours)
        };
        if !announced(&self.connection)
            && (saving || self.connection.documents_open_since(&session, mark))
        {
            let deadline = tokio::time::Instant::now() + ANNOUNCE_WAIT;
            let waited = self
                .connection
                .await_navigation(deadline, |connection| {
                    announced(connection)
                        || (!saving && !connection.documents_open_since(&session, mark))
                })
                .await;
            if waited.is_err() {
                return;
            }
        }
        if !announced(&self.connection) {
            return;
        }
        let deadline = tokio::time::Instant::now() + SAVE_WAIT;
        self.connection
            .await_navigation(deadline, |connection| {
                connection
                    .downloads_since(mark)
                    .iter()
                    .filter(|download| ours(download))
                    .all(|download| download.state != DownloadState::InProgress)
            })
            .await
            .ok();
        let saved = self
            .connection
            .downloads_since(mark)
            .iter()
            .filter(|download| ours(download) && download.state != DownloadState::Canceled)
            .cloned()
            .collect::<Vec<_>>();
        for download in &saved {
            self.downloads.keep(download);
        }
    }
}

#[cfg(test)]
mod tests;
