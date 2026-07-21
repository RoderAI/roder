use anyhow::Context;
use tokio::sync::Mutex;

use crate::client::BlaxelClient;

/// Owns at most one bounded keep-alive process for a session. Serializing
/// replacement avoids concurrent runner operations leaving multiple grace
/// processes behind. A remote lease remains bounded even if Roder exits.
pub(crate) struct StandbyGrace {
    duration_seconds: Option<u64>,
    active_process: Mutex<Option<String>>,
}

impl StandbyGrace {
    pub(crate) fn new(duration_seconds: Option<u64>) -> Self {
        Self {
            duration_seconds,
            active_process: Mutex::new(None),
        }
    }

    pub(crate) async fn refresh(
        &self,
        client: &BlaxelClient,
        endpoint: &str,
    ) -> anyhow::Result<()> {
        let Some(duration_seconds) = self.duration_seconds else {
            return Ok(());
        };
        let mut active = self.active_process.lock().await;
        cancel_active(client, endpoint, &mut active).await?;

        let name = format!("roder-standby-grace-{}", uuid::Uuid::new_v4().simple());
        let process = client
            .start_keep_alive_process(endpoint, &name, duration_seconds)
            .await
            .context("start blaxel standby grace")?;
        anyhow::ensure!(
            !process.is_terminal(),
            "blaxel standby grace process terminated immediately"
        );
        *active = Some(name);
        Ok(())
    }

    pub(crate) async fn cancel(&self, client: &BlaxelClient, endpoint: &str) -> anyhow::Result<()> {
        let mut active = self.active_process.lock().await;
        cancel_active(client, endpoint, &mut active).await
    }
}

async fn cancel_active(
    client: &BlaxelClient,
    endpoint: &str,
    active: &mut Option<String>,
) -> anyhow::Result<()> {
    let Some(name) = active.take() else {
        return Ok(());
    };
    if let Err(error) = client.kill_process(endpoint, &name).await {
        *active = Some(name);
        return Err(error).context("stop previous blaxel standby grace");
    }
    Ok(())
}
