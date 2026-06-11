//! Sprites checkpoint list/restore support. Checkpoint creation lives on
//! `SpritesClient::snapshot`; this module adds the read/restore side used by
//! `restore_checkpoint_id` session config.

use anyhow::Context;
use serde::Deserialize;

use crate::client::SpritesClient;

pub fn is_checkpoint_restore_supported() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SpriteCheckpoint {
    pub id: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default, alias = "created_at")]
    pub created_at: Option<String>,
}

impl SpritesClient {
    pub async fn list_checkpoints(
        &self,
        sprite_name: &str,
    ) -> anyhow::Result<Vec<SpriteCheckpoint>> {
        let response = self
            .http
            .get(self.sprite_url(sprite_name, "/checkpoints"))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("list sprites runner checkpoints")?;
        self.decode_json(response, "list checkpoints").await
    }

    /// Restores the sprite filesystem from a checkpoint (NDJSON progress).
    pub async fn restore_checkpoint(
        &self,
        sprite_name: &str,
        checkpoint_id: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !checkpoint_id.trim().is_empty(),
            "checkpoint id is required for restore"
        );
        let response = self
            .http
            .post(self.sprite_url(
                sprite_name,
                &format!(
                    "/checkpoints/{}/restore",
                    urlencoding::encode(checkpoint_id)
                ),
            ))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("restore sprites runner checkpoint")?;
        self.decode_ndjson_for_errors(response, "restore checkpoint")
            .await
            .map(|_| ())
    }
}
