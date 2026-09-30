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

pub(crate) fn target(
    thread: &str,
    id: Option<u64>,
    targets: Vec<CdpTarget>,
) -> Result<CdpTarget, String> {
    list(thread, &targets);
    let mut tabs = state().lock().unwrap();
    let target = targets
        .into_iter()
        .find(|target| match id {
            Some(id) => tabs.ids.get(&target.id) == Some(&id),
            None => tabs.bound.get(thread) == Some(&target.id),
        })
        .ok_or_else(|| {
            "Desktop browser tab is no longer available; list tabs and choose a tabId".to_string()
        })?;
    tabs.bound.insert(thread.to_string(), target.id.clone());
    Ok(target)
}
