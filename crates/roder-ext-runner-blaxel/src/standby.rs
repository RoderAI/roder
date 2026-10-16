use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use tokio::sync::Mutex;

use crate::client::BlaxelClient;

/// Owns at most one bounded keep-alive process for a session. Serializing
/// replacement avoids concurrent runner operations leaving multiple grace
/// processes behind. A remote lease remains bounded even if Roder exits.
pub(crate) struct StandbyGrace {
    duration_seconds: Option<u64>,
    active_process: Mutex<Option<ActiveGrace>>,
}

struct ActiveGrace {
    name: String,
    started: Instant,
}

/// A lease that started less than a quarter of its duration ago still keeps
/// the sandbox active for most of the grace, so back-to-back operations keep
/// it instead of stopping and restarting it (two round trips each).
fn still_fresh(started: Instant, duration_seconds: u64, now: Instant) -> bool {
    now.saturating_duration_since(started) < Duration::from_secs(duration_seconds) / 4
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
        if active
            .as_ref()
            .is_some_and(|grace| still_fresh(grace.started, duration_seconds, Instant::now()))
        {
            return Ok(());
        }
        cancel_active(client, endpoint, &mut active).await?;

        let name = format!("roder-standby-grace-{}", uuid::Uuid::new_v4().simple());
        let started = Instant::now();
        let process = client
            .start_keep_alive_process(endpoint, &name, duration_seconds)
            .await
            .context("start blaxel standby grace")?;
        anyhow::ensure!(
            !process.is_terminal(),
            "blaxel standby grace process terminated immediately"
        );
        *active = Some(ActiveGrace { name, started });
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
    active: &mut Option<ActiveGrace>,
) -> anyhow::Result<()> {
    let Some(grace) = active.take() else {
        return Ok(());
    };
    if let Err(error) = client.kill_process(endpoint, &grace.name).await {
        *active = Some(grace);
        return Err(error).context("stop previous blaxel standby grace");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lease_stays_fresh_for_a_quarter_of_its_duration() {
        let started = Instant::now();
        assert!(still_fresh(started, 300, started));
        assert!(still_fresh(started, 300, started + Duration::from_secs(74)));
        assert!(!still_fresh(
            started,
            300,
            started + Duration::from_secs(75)
        ));
        // A one-second grace is too short to reuse.
        assert!(!still_fresh(
            started,
            1,
            started + Duration::from_millis(250)
        ));
    }
}
