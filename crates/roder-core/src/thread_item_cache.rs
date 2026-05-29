use std::collections::{HashMap, HashSet, VecDeque};

use roder_api::events::{ThreadId, TurnId};
use roder_api::thread::{
    ThreadItem, ThreadItemDelta, ThreadItemEvent, ThreadItemEventKind, ThreadSnapshot,
};

const THREAD_ITEM_CACHE_MAX_THREADS: usize = 256;

#[derive(Debug, Default)]
pub(crate) struct ThreadItemCache {
    threads: HashMap<ThreadId, ThreadItemCacheEntry>,
    lru: VecDeque<ThreadId>,
}

#[derive(Debug)]
pub(crate) struct ThreadItemCacheEntry {
    current_reasoning_item_ids: HashMap<TurnId, String>,
    next_item_event_seq: u64,
    item_ids: HashSet<(TurnId, String)>,
    transcript_item_counts: HashMap<TurnId, usize>,
}

impl Default for ThreadItemCacheEntry {
    fn default() -> Self {
        Self {
            current_reasoning_item_ids: HashMap::new(),
            next_item_event_seq: 1,
            item_ids: HashSet::new(),
            transcript_item_counts: HashMap::new(),
        }
    }
}

impl ThreadItemCacheEntry {
    pub(crate) fn from_snapshot(snapshot: Option<&ThreadSnapshot>) -> Self {
        let Some(snapshot) = snapshot else {
            return Self::default();
        };
        let next_item_event_seq = snapshot
            .item_events
            .last()
            .map(|event| event.seq)
            .unwrap_or(0)
            .saturating_add(1);
        let item_ids = snapshot
            .item_events
            .iter()
            .map(|event| {
                (
                    event.turn_id.clone(),
                    thread_item_event_kind_item_id(&event.event).to_string(),
                )
            })
            .collect();
        let mut current_reasoning_item_ids = HashMap::new();
        for event in &snapshot.item_events {
            if thread_item_event_kind_is_reasoning(&event.event) {
                current_reasoning_item_ids.insert(
                    event.turn_id.clone(),
                    thread_item_event_kind_item_id(&event.event).to_string(),
                );
            } else {
                current_reasoning_item_ids.remove(&event.turn_id);
            }
        }
        let transcript_item_counts = snapshot
            .turns
            .iter()
            .map(|turn| (turn.turn_id.clone(), turn.items.len()))
            .collect();

        Self {
            current_reasoning_item_ids,
            next_item_event_seq,
            item_ids,
            transcript_item_counts,
        }
    }
}

impl ThreadItemCache {
    pub(crate) fn contains_thread(&self, thread_id: &ThreadId) -> bool {
        self.threads.contains_key(thread_id)
    }

    pub(crate) fn ensure_thread(&mut self, thread_id: &ThreadId, entry: ThreadItemCacheEntry) {
        if !self.threads.contains_key(thread_id) {
            self.threads.insert(thread_id.clone(), entry);
        }
        self.touch(thread_id);
        self.evict_excess_threads();
    }

    pub(crate) fn remove_thread(&mut self, thread_id: &ThreadId) {
        self.threads.remove(thread_id);
        self.lru
            .retain(|cached_thread_id| cached_thread_id != thread_id);
    }

    pub(crate) fn next_item_event_seq(&mut self, thread_id: &ThreadId) -> u64 {
        self.touch(thread_id);
        self.threads
            .get(thread_id)
            .map(|entry| entry.next_item_event_seq)
            .unwrap_or(1)
    }

    pub(crate) fn remember_item_event(&mut self, item_event: &ThreadItemEvent) {
        self.ensure_thread(&item_event.thread_id, ThreadItemCacheEntry::default());
        if let Some(entry) = self.threads.get_mut(&item_event.thread_id) {
            entry.next_item_event_seq = entry
                .next_item_event_seq
                .max(item_event.seq.saturating_add(1));
            entry.item_ids.insert((
                item_event.turn_id.clone(),
                thread_item_event_kind_item_id(&item_event.event).to_string(),
            ));
            if thread_item_event_kind_is_reasoning(&item_event.event) {
                entry.current_reasoning_item_ids.insert(
                    item_event.turn_id.clone(),
                    thread_item_event_kind_item_id(&item_event.event).to_string(),
                );
            } else {
                entry.current_reasoning_item_ids.remove(&item_event.turn_id);
            }
        }
    }

    pub(crate) fn current_reasoning_item_id(
        &mut self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
    ) -> Option<String> {
        self.touch(thread_id);
        self.threads
            .get(thread_id)
            .and_then(|entry| entry.current_reasoning_item_ids.get(turn_id).cloned())
    }

    pub(crate) fn thread_item_exists(
        &mut self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        item_id: &str,
    ) -> bool {
        self.touch(thread_id);
        self.threads.get(thread_id).is_some_and(|entry| {
            entry
                .item_ids
                .contains(&(turn_id.clone(), item_id.to_string()))
        })
    }

    pub(crate) fn latest_transcript_item_index(
        &mut self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
    ) -> Option<usize> {
        self.touch(thread_id);
        self.threads
            .get(thread_id)
            .and_then(|entry| entry.transcript_item_counts.get(turn_id).copied())
            .and_then(|count| count.checked_sub(1))
    }

    pub(crate) fn next_transcript_item_index(
        &mut self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
    ) -> usize {
        self.touch(thread_id);
        self.threads
            .get(thread_id)
            .and_then(|entry| entry.transcript_item_counts.get(turn_id).copied())
            .unwrap_or(0)
    }

    pub(crate) fn remember_transcript_item_index(
        &mut self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        item_index: usize,
    ) {
        self.ensure_thread(thread_id, ThreadItemCacheEntry::default());
        if let Some(entry) = self.threads.get_mut(thread_id) {
            let next_count = item_index.saturating_add(1);
            entry
                .transcript_item_counts
                .entry(turn_id.clone())
                .and_modify(|count| *count = (*count).max(next_count))
                .or_insert(next_count);
        }
    }

    fn touch(&mut self, thread_id: &ThreadId) {
        self.lru
            .retain(|cached_thread_id| cached_thread_id != thread_id);
        self.lru.push_back(thread_id.clone());
    }

    fn evict_excess_threads(&mut self) {
        while self.threads.len() > THREAD_ITEM_CACHE_MAX_THREADS {
            let Some(thread_id) = self.lru.pop_front() else {
                break;
            };
            self.threads.remove(&thread_id);
        }
    }
}

fn thread_item_event_kind_item_id(event: &ThreadItemEventKind) -> &str {
    match event {
        ThreadItemEventKind::ItemStarted { item } => item.id(),
        ThreadItemEventKind::ItemDelta { item_id, .. } => item_id,
        ThreadItemEventKind::ItemCompleted { item } => item.id(),
    }
}

fn thread_item_event_kind_is_reasoning(event: &ThreadItemEventKind) -> bool {
    match event {
        ThreadItemEventKind::ItemStarted { item } | ThreadItemEventKind::ItemCompleted { item } => {
            matches!(item, ThreadItem::Reasoning { .. })
        }
        ThreadItemEventKind::ItemDelta { delta, .. } => matches!(
            delta,
            ThreadItemDelta::ReasoningText { .. }
                | ThreadItemDelta::ReasoningSummaryPartAdded { .. }
                | ThreadItemDelta::ReasoningSummaryText { .. }
        ),
    }
}
