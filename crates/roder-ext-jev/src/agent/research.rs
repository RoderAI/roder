//! Explicit native document reads share the direct tools' extraction and budgets.
use super::*;
#[derive(Default)]
pub(super) struct Research {
    source: Value,
    document: Option<Value>,
    offset: usize,
    next: Option<usize>,
}
impl Research {
    pub(super) fn offer(&mut self, observation: &mut Value) {
        let source = json!([observation["url"], observation["marker"][0]]);
        if self.source != source {
            *self = Self {
                source,
                ..Default::default()
            };
        }
        let label = if self.document.is_none() {
            "Read rendered document text and source links without scrolling"
        } else {
            "Read the next page of rendered document text"
        };
        if (self.document.is_none() || self.next.is_some())
            && let Some(actions) = observation["actions"].as_array_mut()
        {
            actions.push(json!({"id":"read_document","kind":"read_document","label":label}));
        }
    }
}
impl Agent {
    pub(super) async fn read_document(&mut self, action: &Value) -> anyhow::Result<()> {
        self.read_document_page(action).await?;
        self.observe().await?;
        self.check_scope()
    }
    pub(super) async fn read_document_page(&mut self, action: &Value) -> anyhow::Result<()> {
        if self.research.document.is_none() {
            let Some(mut document) = self.browser.document_text().await? else {
                return Err(
                    JevStop::new(JevStatus::Blocked, "Document reading unavailable").into(),
                );
            };
            self.secrets.scrub_observation(&mut document);
            if document["url"] != self.observation["url"] {
                return Err(StaleObservation::new("Document changed before reading").into());
            }
            self.research.document = Some(document);
        }
        let document = self.research.document.as_ref().unwrap();
        let full = self
            .secrets
            .scrub(document["text"].as_str().unwrap_or_default());
        let offset = self.research.next.unwrap_or(self.research.offset);
        let excerpt = full.chars().skip(offset).take(6000).collect::<String>();
        let shown = excerpt.split_inclusive('\n').take(100).collect::<String>();
        let next = offset + shown.chars().count();
        self.research.offset = next;
        self.research.next = (next < full.chars().count()).then_some(next);
        let mut page = json!({"url":document["url"],"text":shown});
        if document["limited"] == true {
            page["text"] = json!(format!(
                "{}\nDocument extraction capped; omitted text is not evidence of absence.",
                shown
            ));
        }
        self.evidence.remember(&page);
        let elapsed = self.elapsed();
        self.history.push(json!({"step":self.history.len()+1,"action":action["label"],"kind":"read_document","choice":action["id"],"url":page["url"],"page_changed":false,"effect":format!("Read document characters {offset}–{next}; facts retained as source evidence"),"elapsed_ms":elapsed,"executed_ms":elapsed}));
        Ok(())
    }
}

impl Agent {
    pub(super) async fn offer_research(&mut self) -> anyhow::Result<()> {
        if !self.browser.document_read_available() {
            return Ok(());
        }
        self.research.offer(&mut self.observation);
        if self.config.research
            && self.research.document.is_none()
            && self.history.iter().filter(chosen).count() < self.config.max_actions
        {
            let action = json!({"id":"read_document","label":"Read document on arrival"});
            self.read_document_page(&action).await?;
            if let Some(actions) = self.observation["actions"].as_array_mut() {
                actions.retain(|a| a["kind"] != "read_document");
            }
            self.research.offer(&mut self.observation);
        }
        Ok(())
    }
}
