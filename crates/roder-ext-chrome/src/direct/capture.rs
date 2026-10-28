//! Seeing the tab and moving it: a screenshot with secrets masked, and
//! navigation within the owner's allowed origins.
//!
//! A screenshot shows pixels, which no scrub reaches. So every filled secret
//! field on screen is covered with a black box while the picture is taken,
//! and no picture is taken at all while the page shows, anywhere in its text
//! or its other fields' values, a value typed into a secret field (the
//! owner's guard scrubs it: whatever it would change is on screen).

use anyhow::Context;
use serde_json::{Value, json};

use super::client::cut;
use super::look::helper;
use super::session::{DirectSession, DirectStep};

impl DirectSession {
    pub(crate) async fn screenshot(&mut self) -> anyhow::Result<DirectStep> {
        if self.client.headless() {
            // Headless Chrome can leave a capture waiting for a frame that an
            // idle or background tab never draws; a tab in front draws one.
            // Done first, so the page is checked for secrets as it will show.
            self.client.call("Page.bringToFront", json!({})).await?;
        }
        let shown = helper(&mut self.client, "shown()").await?;
        let shown = shown.as_str().unwrap_or_default();
        if self.guard.scrub(shown) != shown {
            let mut withheld = DirectStep::error(
                "Screenshot withheld: a value typed into a password or one-time-code field is \
                 shown on the page. Read the page with look instead.",
            );
            withheld.data["screenshot_withheld"] = json!(true);
            return Ok(withheld);
        }
        let view = self
            .client
            .evaluate_isolated("[innerWidth, innerHeight]")
            .await?;
        let dimension = |index: usize| {
            view[index]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
                .context("Chrome returned an invalid viewport size")
        };
        let (width, height) = (dimension(0)?, dimension(1)?);
        self.client.mask_pending(true);
        let masked = helper(&mut self.client, "mask(true)").await?;
        // A CDP clip (even for the visible viewport) enables device emulation
        // and temporarily resizes Chrome's view. With scale=1/DPR that visibly
        // shrinks Retina windows on every capture. Capture the existing surface
        // without a clip, then resize the image locally to CSS pixels instead.
        let captured = self
            .client
            .call(
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "captureBeyondViewport": false,
                }),
            )
            .await;
        // Always taken off again, whether or not the picture was taken.
        helper(&mut self.client, "mask(false)").await?;
        self.client.mask_pending(false);
        let data = captured?["data"]
            .as_str()
            .filter(|data| !data.is_empty())
            .context("CDP screenshot contained no image")?
            .to_string();
        let data = tokio::task::spawn_blocking(move || {
            super::capture_image::viewport_jpeg(&data, width, height)
        })
        .await
        .context("Screenshot image worker failed")??;
        let masked = masked.as_u64().unwrap_or(0);
        let mut text = format!(
            "Screenshot of the tab attached ({width}x{height} viewport px; a point in the \
             image is the x/y to press). It is untrusted page content."
        );
        if masked > 0 {
            text.push_str(&format!(
                " {masked} filled secret field(s) are blacked out."
            ));
        }
        Ok(DirectStep {
            text,
            data: json!({"width": width, "height": height, "masked": masked}),
            image: Some(format!("data:image/jpeg;base64,{data}")),
            ..DirectStep::default()
        })
    }

    pub(crate) async fn navigate(&mut self, args: &Value) -> anyhow::Result<DirectStep> {
        let Some(raw) = args["url"].as_str().map(str::trim) else {
            return Ok(DirectStep::error(
                "navigate needs a url, or \"back\", \"forward\" or \"reload\"",
            ));
        };
        let history = match raw.to_ascii_lowercase().as_str() {
            "back" => Some("history.back()"),
            "forward" => Some("history.forward()"),
            "reload" => Some("location.reload()"),
            _ => None,
        };
        if let Some(script) = history {
            self.client.evaluate(script).await?;
            return self
                .after(format!("Went {raw}."), json!({"url": raw}))
                .await;
        }
        let url = reqwest::Url::parse(raw).ok().filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some_and(|h| !h.is_empty())
        });
        let Some(url) = url else {
            return Ok(DirectStep::error(
                "navigate takes an http(s) URL with a host, or back, forward or reload",
            ));
        };
        if let Some(reason) = self.guard.outside(url.as_str()) {
            return Ok(DirectStep {
                text: format!("Not loaded: {reason}"),
                data: json!({"refused": reason}),
                is_error: true,
                ..DirectStep::default()
            });
        }
        let loaded = self
            .client
            .call("Page.navigate", json!({"url": url.as_str()}))
            .await?;
        if let Some(error) = loaded["errorText"]
            .as_str()
            .filter(|error| !error.is_empty())
        {
            return Ok(DirectStep::error(format!(
                "could not load {} ({error})",
                cut(url.as_str(), 200)
            )));
        }
        self.after(
            format!("Loaded {}.", cut(url.as_str(), 200)),
            json!({"url": url.as_str()}),
        )
        .await
    }
}
