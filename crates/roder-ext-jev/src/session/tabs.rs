//! The tabs one session owns between calls, by target id.
//!
//! Each tab gets a short id (`t1`, `t2`, …) for the caller, kept for as long
//! as the tab is. The last one is current. After a call, the page's
//! [`TabLedger`] says which tabs it ended with (a tab an action opened is
//! adopted, a tab that closed itself is gone); tabs its tabs opened that it
//! never adopted are closed; and the oldest tabs past the cap are closed,
//! never the current one or the tab that opened it.

use serde::Serialize;

use crate::page::{OwnedTab, TabLedger};

/// One tab the session owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TabRecord {
    /// `t1`, `t2`, …, as the caller sees it.
    pub(crate) id: String,
    #[serde(skip)]
    pub(crate) target: String,
    /// The id of the session tab that opened this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) opened_by: Option<String>,
}

/// How a call came to be on the tab it acted in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TabNote {
    /// A tab opened for this call: the session's first, after a reset, or
    /// asked for with `tab: "new"`.
    New,
    /// The current tab, on the page the last call left it on.
    Continued,
    /// The current tab, loading the call's url.
    Navigated,
    /// The current tab, loading the call's url although the call asked for
    /// a new tab, because the current one held nothing worth keeping; why.
    Reused(String),
    /// The tab the last call used is gone; why, and where the call went on.
    Reopened(String),
}

impl TabNote {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Continued => "continued",
            Self::Navigated | Self::Reused(_) => "navigated",
            Self::Reopened(_) => "reopened",
        }
    }

    pub(crate) fn detail(&self) -> Option<&str> {
        match self {
            Self::Reopened(reason) | Self::Reused(reason) => Some(reason),
            _ => None,
        }
    }
}

/// The tabs a session owns, oldest first, the last current.
#[derive(Debug, Clone)]
pub(crate) struct SessionTabs {
    pub(crate) records: Vec<TabRecord>,
    next: u32,
    /// Where the last call left the current tab.
    pub(crate) last_url: Option<String>,
    pub(crate) max_tabs: usize,
}

impl SessionTabs {
    pub(crate) fn new(max_tabs: usize) -> Self {
        Self {
            records: Vec::new(),
            next: 1,
            last_url: None,
            max_tabs: max_tabs.max(1),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub(crate) fn current(&self) -> Option<&TabRecord> {
        self.records.last()
    }

    pub(crate) fn targets(&self) -> Vec<String> {
        self.records
            .iter()
            .map(|record| record.target.clone())
            .collect()
    }

    /// The tabs as a page re-attaches to them.
    pub(crate) fn owned(&self) -> Vec<OwnedTab> {
        self.records
            .iter()
            .map(|record| OwnedTab {
                target: record.target.clone(),
                opener: record
                    .opened_by
                    .as_ref()
                    .and_then(|id| self.by_id(id))
                    .map(|opener| opener.target.clone()),
            })
            .collect()
    }

    fn by_id(&self, id: &str) -> Option<&TabRecord> {
        self.records.iter().find(|record| record.id == id)
    }

    /// The id of the tab with `target`.
    pub(crate) fn id_of(&self, target: &str) -> Option<&str> {
        self.records
            .iter()
            .find(|record| record.target == target)
            .map(|record| record.id.as_str())
    }

    /// Drop the tabs with these targets, which Chrome no longer has.
    pub(crate) fn forget(&mut self, targets: &[String]) {
        self.records
            .retain(|record| !targets.contains(&record.target));
    }

    /// Drop every tab and where the last call left off; ids keep counting.
    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.last_url = None;
    }

    /// Take the tabs a call's page ended with. `beside` keeps the tabs the
    /// page never re-attached to (a call with `tab: "new"`); otherwise the
    /// page's tabs are the session's. Returns the stray tabs to close.
    pub(crate) fn absorb(&mut self, ledger: &TabLedger, beside: bool) -> Vec<String> {
        let previous = std::mem::take(&mut self.records);
        let mut records = if beside { previous.clone() } else { Vec::new() };
        for owned in &ledger.tabs {
            let known = previous
                .iter()
                .find(|record| record.target == owned.target)
                .cloned();
            let record = match known {
                Some(record) => record,
                None => {
                    let opened_by = owned.opener.as_ref().and_then(|opener| {
                        records
                            .iter()
                            .chain(&previous)
                            .find(|record| &record.target == opener)
                            .map(|record| record.id.clone())
                    });
                    let id = format!("t{}", self.next);
                    self.next += 1;
                    TabRecord {
                        id,
                        target: owned.target.clone(),
                        opened_by,
                    }
                }
            };
            records.retain(|kept| kept.target != record.target);
            records.push(record);
        }
        self.records = records;
        ledger
            .stray
            .iter()
            .filter(|stray| self.id_of(stray).is_none())
            .cloned()
            .collect()
    }

    /// Adopt a tab one of the session's tabs opened, outside a Jev call (the
    /// full browser tools followed it); it becomes current.
    pub(crate) fn adopt(&mut self, target: &str, opener: &str) {
        if self.id_of(target).is_some() {
            return;
        }
        let opened_by = self.id_of(opener).map(str::to_string);
        let id = format!("t{}", self.next);
        self.next += 1;
        self.records.push(TabRecord {
            id,
            target: target.to_string(),
            opened_by,
        });
    }

    /// Drop the oldest tabs past the cap, never the current one or the tab
    /// that opened it, and return their targets to close.
    pub(crate) fn cap(&mut self) -> Vec<String> {
        let mut closed = Vec::new();
        while self.records.len() > self.max_tabs {
            let Some(current) = self.current().cloned() else {
                break;
            };
            let spare = self.records.iter().position(|record| {
                record.id != current.id && Some(&record.id) != current.opened_by.as_ref()
            });
            let Some(spare) = spare else {
                break;
            };
            closed.push(self.records.remove(spare).target);
        }
        closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger(tabs: &[(&str, Option<&str>)], stray: &[&str]) -> TabLedger {
        TabLedger {
            tabs: tabs
                .iter()
                .map(|(target, opener)| OwnedTab {
                    target: target.to_string(),
                    opener: opener.map(str::to_string),
                })
                .collect(),
            stray: stray.iter().map(|stray| stray.to_string()).collect(),
        }
    }

    fn ids(tabs: &SessionTabs) -> Vec<(String, Option<String>)> {
        tabs.records
            .iter()
            .map(|record| (record.id.clone(), record.opened_by.clone()))
            .collect()
    }

    #[test]
    fn ids_follow_their_tabs_across_calls() {
        let mut tabs = SessionTabs::new(3);
        assert!(tabs.absorb(&ledger(&[("A", None)], &[]), false).is_empty());
        // A popup A opened is adopted; a second one it opened was not.
        let stray = tabs.absorb(&ledger(&[("A", None), ("B", Some("A"))], &["C"]), false);
        assert_eq!(stray, ["C"]);
        assert_eq!(
            ids(&tabs),
            [("t1".into(), None), ("t2".into(), Some("t1".into()))]
        );
        assert_eq!(tabs.current().unwrap().target, "B");
        assert_eq!(tabs.owned()[1].opener.as_deref(), Some("A"));
        // The popup closed itself: back on A, still t1.
        tabs.absorb(&ledger(&[("A", None)], &[]), false);
        assert_eq!(ids(&tabs), [("t1".into(), None)]);
        // A new tab beside it keeps A, and ids never repeat.
        tabs.absorb(&ledger(&[("D", None)], &[]), true);
        assert_eq!(ids(&tabs), [("t1".into(), None), ("t3".into(), None)]);
    }

    #[test]
    fn the_cap_closes_the_oldest_but_never_the_current_tab_or_its_opener() {
        let mut tabs = SessionTabs::new(3);
        for target in ["A", "B", "C", "D"] {
            tabs.absorb(&ledger(&[(target, None)], &[]), true);
        }
        assert_eq!(tabs.cap(), ["A"]);
        assert_eq!(tabs.targets(), ["B", "C", "D"]);
        // E, opened by B, becomes current: B stays, C goes.
        tabs.absorb(
            &ledger(
                &[("B", None), ("C", None), ("D", None), ("E", Some("B"))],
                &[],
            ),
            false,
        );
        assert_eq!(tabs.cap(), ["C"]);
        assert_eq!(tabs.targets(), ["B", "D", "E"]);
        let mut one = SessionTabs::new(1);
        one.absorb(&ledger(&[("A", None), ("B", Some("A"))], &[]), false);
        // Nothing can go: B is current and A opened it.
        assert!(one.cap().is_empty());
    }

    #[test]
    fn forgetting_and_clearing() {
        let mut tabs = SessionTabs::new(3);
        tabs.absorb(&ledger(&[("A", None), ("B", Some("A"))], &[]), false);
        tabs.last_url = Some("https://a.test/".into());
        tabs.forget(&["B".into()]);
        assert_eq!(tabs.targets(), ["A"]);
        tabs.clear();
        assert!(tabs.is_empty() && tabs.last_url.is_none());
        tabs.absorb(&ledger(&[("C", None)], &[]), false);
        assert_eq!(tabs.current().unwrap().id, "t3");
    }
}
