//! Getting the value a fill types: from the configured resolver (a
//! supervisor's, or Roder's text helper), never guessed. A secret field's
//! value is recorded as [`SECRET`] wherever the run keeps it.

use serde_json::{Value, json};

use super::Agent;
use crate::engine::{JevStatus, JevStop, JevTextValue, StaleObservation};
use crate::secret::{self, SECRET};
use crate::text_helper;
use crate::usage::JevBilled;

impl Agent {
    /// Typing needs a value, and only the text resolver may supply one.
    pub(super) async fn write_text_if_needed(
        &mut self,
        action: &Value,
    ) -> anyhow::Result<Option<JevTextValue>> {
        if action["kind"].as_str() != Some("fill") {
            return Ok(None);
        }
        if !self.browser.fresh(&self.observation, Some(action)).await? {
            return Err(StaleObservation::new(
                "Page changed before text generation. Choose again.",
            )
            .into());
        }
        let context = text_helper::field_context(
            &self.config.goal,
            action,
            &self.observation,
            &self.history,
            chrono::Local::now().date_naive(),
        );
        if let Some((pending, written)) = &self.pending_text
            && pending == &context
        {
            return Ok(Some(written.clone()));
        }
        let Some(text) = self.config.text.as_ref() else {
            return Err(JevStop::new(
                JevStatus::NeedsInput,
                format!(
                    "TYPE_TEXT into {:?} needs a text model; sign in with `roder auth login \
                     codex`, configure a Roder chat-completions provider or set \
                     JEV_TEXT_MODEL_API_KEY. No text is guessed.",
                    action["label"].as_str().unwrap_or_default()
                ),
            )
            .into());
        };
        let secret = secret::is_secret(action);
        let written = match text.resolve(&context).await {
            Ok(written) => written,
            Err(error) => {
                // A reply with no usable value was still billed.
                if let Some(usage) = JevBilled::usage_of(&error) {
                    self.text_calls.push(json!({
                        "usage": usage,
                        "field": action["label"],
                        "value": null,
                    }));
                }
                return Err(error);
            }
        };
        self.pending_text = Some((context, written.clone()));
        self.text_calls.push(json!({
            "model": written.model,
            "latency_ms": written.latency_ms,
            "usage": written.usage,
            "field": action["label"],
            "value": if secret { json!(SECRET) } else { json!(written.value) },
        }));
        Ok(Some(written))
    }
}
