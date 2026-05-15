use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use roder_api::events::{EventEnvelope, ThreadId};
use roder_api::session::{SessionStore, SessionStoreFactory};

pub struct JsonlSessionStore {
    pub base_path: PathBuf,
}

#[async_trait::async_trait]
impl SessionStore for JsonlSessionStore {
    async fn append_event(&self, thread_id: &ThreadId, envelope: &EventEnvelope) -> anyhow::Result<()> {
        let dir = self.base_path.join(thread_id);
        fs::create_dir_all(&dir).await?;
        
        let file_path = dir.join("events.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await?;

        let json = serde_json::to_string(envelope)?;
        file.write_all(format!("{}\n", json).as_bytes()).await?;
        Ok(())
    }

    async fn load_events(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<EventEnvelope>> {
        let file_path = self.base_path.join(thread_id).join("events.jsonl");
        if !file_path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&file_path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        
        let mut events = Vec::new();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let envelope: EventEnvelope = serde_json::from_str(&line)?;
            events.push(envelope);
        }

        Ok(events)
    }
}

pub struct JsonlSessionStoreFactory {
    pub base_path: PathBuf,
}

impl SessionStoreFactory for JsonlSessionStoreFactory {
    fn create(&self) -> Arc<dyn SessionStore> {
        Arc::new(JsonlSessionStore {
            base_path: self.base_path.clone(),
        })
    }
}
