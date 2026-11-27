//! Stable numeric ids for Desktop targets and a default binding per thread.
//! CDP target enumeration order is not tab identity.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};

use crate::desktop_cdp::CdpTarget;

#[derive(Default)]
struct Tabs {
    next: u64,
    ids: HashMap<String, u64>,
    bound: HashMap<String, String>,
}

fn state() -> &'static Mutex<Tabs> {
    static TABS: OnceLock<Mutex<Tabs>> = OnceLock::new();
    TABS.get_or_init(|| Mutex::new(Tabs::default()))
}

pub(crate) fn list(thread: &str, targets: &[CdpTarget]) -> Value {
    let mut tabs = state().lock().unwrap();
    if let Some(first) = targets.first() {
        tabs.bound
            .entry(thread.to_string())
            .or_insert_with(|| first.id.clone());
    }
    let active = tabs.bound.get(thread).cloned();
    let rows = targets
        .iter()
        .map(|target| {
            let id = if let Some(id) = tabs.ids.get(&target.id) {
                *id
            } else {
                let id = tabs.next;
                tabs.next += 1;
                tabs.ids.insert(target.id.clone(), id);
                id
            };
            json!({"id":id, "targetId":target.id, "title":target.title, "url":target.url,
            "active":Some(&target.id)==active.as_ref(), "browser":"roder-desktop"})
        })
        .collect::<Vec<_>>();
    json!({"tabs":rows})
}

pub(crate) fn bind(thread: &str, target_id: &str) -> u64 {
    let mut tabs = state().lock().unwrap();
    tabs.bound.insert(thread.to_string(), target_id.to_string());
    if let Some(id) = tabs.ids.get(target_id) {
        return *id;
    }
    let id = tabs.next;
    tabs.next += 1;
    tabs.ids.insert(target_id.to_string(), id);
    id
}

pub(crate) fn target(
    thread: &str,
    id: Option<u64>,
    targets: Vec<CdpTarget>,
) -> Result<CdpTarget, String> {
    let targets_empty = targets.is_empty();
    list(thread, &targets);
    let mut tabs = state().lock().unwrap();
    let target = targets
        .into_iter()
        .find(|target| match id {
            Some(id) => tabs.ids.get(&target.id) == Some(&id),
            None => tabs.bound.get(thread) == Some(&target.id),
        })
        .ok_or_else(|| {
            if targets_empty {
                "The Desktop browser has no open page. Open the Browser pane in Roder Desktop, \
                 or pair the Chrome extension with this Roder instance from /chrome."
                    .to_string()
            } else {
                "Desktop browser tab is no longer available; list tabs and choose a tabId"
                    .to_string()
            }
        })?;
    tabs.bound.insert(thread.to_string(), target.id.clone());
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(id: &str) -> CdpTarget {
        serde_json::from_value(json!({
            "id":id, "type":"page", "url":"about:blank",
            "webSocketDebuggerUrl":"ws://127.0.0.1/devtools/page/test"
        }))
        .unwrap()
    }

    #[test]
    fn newly_created_target_gets_a_stable_id_and_becomes_the_thread_default() {
        let old = page("bind-test-old");
        list("bind-test-thread", &[old]);
        let created = bind("bind-test-thread", "bind-test-new");
        assert_eq!(bind("bind-test-thread", "bind-test-new"), created);
        let selected = target(
            "bind-test-thread",
            None,
            vec![page("bind-test-old"), page("bind-test-new")],
        )
        .unwrap();
        assert_eq!(selected.id, "bind-test-new");
    }

    #[test]
    fn an_empty_browser_has_a_different_diagnostic_from_a_stale_tab_id() {
        let empty = target("empty-test-thread", None, vec![]).unwrap_err();
        assert!(empty.contains("Browser pane"));
        assert!(!empty.contains("no longer available"));
        let stale = target(
            "stale-test-thread",
            Some(u64::MAX),
            vec![page("stale-test-page")],
        )
        .unwrap_err();
        assert!(stale.contains("no longer available"));
    }
}
