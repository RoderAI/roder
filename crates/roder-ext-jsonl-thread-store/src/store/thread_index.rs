use std::future::Future;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::*;

pub(super) const THREAD_INDEX_FILE: &str = "thread-index.json";
const THREAD_INDEX_LOCK_FILE: &str = "thread-index.lock";
const THREAD_INDEX_DIRTY_FILE: &str = "thread-index.dirty";
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(2);
const INDEX_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadIndexEntry {
    thread_id: ThreadId,
    updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadIndex {
    #[serde(default)]
    version: u32,
    threads: Vec<ThreadIndexEntry>,
}

pub(super) enum ThreadIndexChange {
    Upsert {
        thread_id: ThreadId,
        updated_at: OffsetDateTime,
    },
    Remove(ThreadId),
    None,
}

impl ThreadIndexChange {
    pub(super) fn upsert(metadata: &ThreadMetadata) -> Self {
        Self::Upsert {
            thread_id: metadata.thread_id.clone(),
            updated_at: metadata.updated_at,
        }
    }
}

impl JsonlThreadStore {
    // Own the transaction independently of the request. Tokio filesystem writes
    // and renames can finish after a cancelled caller drops its future.
    pub(super) async fn with_thread_index_lock<T, F>(
        &self,
        transaction: impl FnOnce(Self) -> F + Send + 'static,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: Future<Output = anyhow::Result<T>> + Send + 'static,
    {
        let store = Self {
            base_path: self.base_path.clone(),
        };
        tokio::spawn(async move {
            let _lock = acquire_thread_index_lock(&store.base_path, LOCK_TIMEOUT).await?;
            transaction(store).await
        })
        .await
        .context("thread index transaction task")?
    }

    pub(super) async fn mutate_thread_index<T, F>(
        &self,
        mutation: impl FnOnce(Self) -> F + Send + 'static,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: Future<Output = anyhow::Result<(T, ThreadIndexChange)>> + Send + 'static,
    {
        self.with_thread_index_lock(move |store| async move {
            let mut index = store.load_or_rebuild_thread_index().await?;
            // The derived index and per-thread metadata use different files.
            // Mark an unfinished update so a later reader can repair either
            // an error or a process exit between those writes.
            let dirty_path = store.base_path.join(THREAD_INDEX_DIRTY_FILE);
            fs::write(&dirty_path, [])
                .await
                .context("mark thread index dirty")?;
            let (result, change) = mutation(Self {
                base_path: store.base_path.clone(),
            })
            .await?;
            match change {
                ThreadIndexChange::Upsert {
                    thread_id,
                    updated_at,
                } => {
                    upsert_thread_index_entry(&mut index, thread_id, updated_at);
                    store.write_thread_index(&index).await?;
                }
                ThreadIndexChange::Remove(thread_id) => {
                    index.threads.retain(|entry| entry.thread_id != thread_id);
                    store.write_thread_index(&index).await?;
                }
                ThreadIndexChange::None => {}
            }
            store.clear_thread_index_dirty().await?;
            Ok(result)
        })
        .await
    }

    pub(super) async fn list_indexed_threads(
        &self,
        options: ThreadListOptions,
    ) -> anyhow::Result<ThreadListPage> {
        if !self.base_path.exists() {
            return Ok(ThreadListPage::default());
        }
        let index = match self.load_thread_index().await {
            Ok(Some(index)) if index.version == INDEX_VERSION => index,
            _ => {
                self.with_thread_index_lock(move |store| async move {
                    store.load_or_rebuild_thread_index().await
                })
                .await?
            }
        };
        let (page, stale) = self.thread_index_page(&index, &options).await?;
        if !stale {
            return Ok(page);
        }
        self.with_thread_index_lock(move |store| async move {
            let index = store.rebuild_thread_index().await?;
            let (page, _) = store.thread_index_page(&index, &options).await?;
            Ok(page)
        })
        .await
    }

    async fn thread_index_page(
        &self,
        index: &ThreadIndex,
        options: &ThreadListOptions,
    ) -> anyhow::Result<(ThreadListPage, bool)> {
        let offset = options
            .cursor
            .as_deref()
            .and_then(|cursor| cursor.parse::<usize>().ok())
            .unwrap_or(0)
            .min(index.threads.len());
        let limit = options
            .limit
            .unwrap_or(index.threads.len().saturating_sub(offset));
        let next_offset = offset.saturating_add(limit).min(index.threads.len());
        let mut threads = Vec::new();
        let mut stale = false;
        for entry in index.threads.iter().skip(offset).take(limit) {
            match self.load_thread_metadata(&entry.thread_id).await? {
                Some(metadata) => threads.push(metadata),
                None => stale = true,
            }
        }
        threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));
        Ok((
            ThreadListPage {
                threads,
                next_cursor: (next_offset < index.threads.len()).then(|| next_offset.to_string()),
                backwards_cursor: (offset > 0).then(|| offset.saturating_sub(limit).to_string()),
            },
            stale,
        ))
    }

    async fn load_or_rebuild_thread_index(&self) -> anyhow::Result<ThreadIndex> {
        match self.load_thread_index().await {
            Ok(Some(index)) if index.version == INDEX_VERSION => Ok(index),
            // Older writers raced even when the index parsed. The canonical
            // thread directories can rebuild that derived data once.
            _ => self.rebuild_thread_index().await,
        }
    }

    fn thread_index_path(&self) -> PathBuf {
        self.base_path.join(THREAD_INDEX_FILE)
    }

    async fn load_thread_index(&self) -> anyhow::Result<Option<ThreadIndex>> {
        if fs::try_exists(self.base_path.join(THREAD_INDEX_DIRTY_FILE)).await? {
            return Ok(None);
        }
        let path = self.thread_index_path();
        let data = match fs::read(&path).await {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| format!("read thread index {}", path.display()));
            }
        };
        serde_json::from_slice(&data)
            .map(Some)
            .with_context(|| format!("parse thread index {}", path.display()))
    }

    async fn write_thread_index(&self, index: &ThreadIndex) -> anyhow::Result<()> {
        let path = self.thread_index_path();
        let serialized = serde_json::to_vec_pretty(index).context("serialize thread index")?;
        fs::create_dir_all(&self.base_path)
            .await
            .with_context(|| format!("create thread root {}", self.base_path.display()))?;
        let tmp_path = path.with_file_name(format!(
            ".{}.{}.tmp",
            THREAD_INDEX_FILE,
            uuid::Uuid::new_v4()
        ));
        fs::write(&tmp_path, serialized)
            .await
            .with_context(|| format!("write thread index temp {}", tmp_path.display()))?;
        if let Err(err) = fs::rename(&tmp_path, &path).await {
            let _ = fs::remove_file(&tmp_path).await;
            return Err(err).with_context(|| format!("replace thread index {}", path.display()));
        }
        Ok(())
    }

    async fn clear_thread_index_dirty(&self) -> anyhow::Result<()> {
        match fs::remove_file(self.base_path.join(THREAD_INDEX_DIRTY_FILE)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("clear thread index dirty marker"),
        }
    }

    async fn rebuild_thread_index(&self) -> anyhow::Result<ThreadIndex> {
        let mut threads = self.scan_thread_index_entries().await?;
        sort_thread_index_entries(&mut threads);
        let index = ThreadIndex {
            version: INDEX_VERSION,
            threads,
        };
        self.write_thread_index(&index).await?;
        self.clear_thread_index_dirty().await?;
        Ok(index)
    }

    async fn scan_thread_index_entries(&self) -> anyhow::Result<Vec<ThreadIndexEntry>> {
        let mut entries = fs::read_dir(&self.base_path)
            .await
            .with_context(|| format!("read thread directory {}", self.base_path.display()))?;
        let mut threads = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("read thread entry under {}", self.base_path.display()))?
        {
            let file_type = entry.file_type().await.with_context(|| {
                format!("read thread entry type under {}", self.base_path.display())
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let thread_id = entry.file_name().to_string_lossy().to_string();
            if thread_id == "discovery-state" || is_thread_directory_without_metadata(&entry.path())
            {
                continue;
            }
            let metadata = self
                .load_or_infer_metadata(&entry.path(), &thread_id)
                .await?;
            threads.push(ThreadIndexEntry {
                thread_id,
                updated_at: metadata.updated_at,
            });
        }
        Ok(threads)
    }
}

async fn acquire_thread_index_lock(
    root: &Path,
    timeout: Duration,
) -> anyhow::Result<std::fs::File> {
    fs::create_dir_all(root)
        .await
        .with_context(|| format!("create thread root {}", root.display()))?;
    let path = root.join(THREAD_INDEX_LOCK_FILE);
    // The inode must remain stable across index replacements and processes.
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .await
        .with_context(|| format!("open thread index lock {}", path.display()))?
        .into_std()
        .await;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("acquire thread index lock {}", path.display()));
            }
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("timed out waiting for thread index lock {}", path.display()),
            )
            .into());
        }
        tokio::time::sleep_until(deadline.min(now + LOCK_RETRY_DELAY)).await;
    }
}

fn upsert_thread_index_entry(
    index: &mut ThreadIndex,
    thread_id: ThreadId,
    updated_at: OffsetDateTime,
) {
    if let Some(entry) = index
        .threads
        .iter_mut()
        .find(|entry| entry.thread_id == thread_id)
    {
        entry.updated_at = updated_at;
    } else {
        index.threads.push(ThreadIndexEntry {
            thread_id,
            updated_at,
        });
    }
    sort_thread_index_entries(&mut index.threads);
}

fn sort_thread_index_entries(entries: &mut [ThreadIndexEntry]) {
    entries.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.thread_id.cmp(&left.thread_id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn cancelled_request_keeps_index_lock_until_owned_transaction_finishes() {
        let root = std::env::temp_dir().join(format!("roder-index-lock-{}", uuid::Uuid::new_v4()));
        let store = JsonlThreadStore {
            base_path: root.clone(),
        };
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let waiter = tokio::spawn(async move {
            store
                .with_thread_index_lock(move |_store| async move {
                    entered_tx.send(()).unwrap();
                    release_rx.await?;
                    Ok(())
                })
                .await
        });
        entered_rx.await.unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        let error = acquire_thread_index_lock(&root, Duration::from_millis(40))
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::TimedOut
        );
        release_tx.send(()).unwrap();
        let guard = acquire_thread_index_lock(&root, Duration::from_secs(1))
            .await
            .unwrap();
        drop(guard);
        std::fs::remove_dir_all(root).unwrap();
    }
}
