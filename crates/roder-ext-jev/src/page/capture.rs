//! Capture the existing viewport and scale the image outside Chrome.
use anyhow::Context;
use roder_ext_chrome::direct::viewport_jpeg;
use serde_json::json;

use super::Page;

impl Page {
    pub(super) async fn capture_screenshot(&mut self) -> anyhow::Result<String> {
        let view = self.evaluate("[innerWidth, innerHeight]").await?;
        let dimension = |index: usize| {
            view[index]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
                .context("Chrome returned an invalid viewport size")
        };
        let (width, height) = (dimension(0)?, dimension(1)?);
        let capture = self
            .call(
                "Page.captureScreenshot",
                json!({"format":"png", "captureBeyondViewport":false}),
            )
            .await?;
        let data = capture["data"]
            .as_str()
            .filter(|data| !data.is_empty())
            .context("Chrome returned no screenshot")?
            .to_string();
        let data = tokio::task::spawn_blocking(move || viewport_jpeg(&data, width, height))
            .await
            .context("Screenshot image worker failed")??;
        Ok(format!("data:image/jpeg;base64,{data}"))
    }
}
