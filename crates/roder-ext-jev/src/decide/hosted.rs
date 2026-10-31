//! Hosted Jev decision packing and on-demand context pages.
use super::asking::Shed;
use super::context_budget::{self, Bounded};
use super::context_pages::Pages;
use super::*;
use std::sync::Mutex;

impl JevTypeSafeDecisionClient {
    pub(super) async fn decide(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
        gate: bool,
    ) -> anyhow::Result<JevDecision> {
        if self.composed_transport {
            return self
                .decide_view(observation, goal, history, gate, self.transport.as_ref())
                .await;
        }
        let leaf = crate::request_budget::Transport(self.transport.as_ref());
        let reports = Mutex::new(Vec::new());
        let image_cost = observation["_screenshot"]
            .as_str()
            .filter(|_| self.images)
            .and_then(|s| images::fit(s).ok())
            .map_or(0, |f| {
                (f.width as usize).div_ceil(14) * (f.height as usize).div_ceil(14) * 2
            });
        let bounded = Bounded {
            inner: &leaf,
            image_cost,
            reports: &reports,
        };
        let open = super::asking::Open(&bounded);
        let mut pages = Pages::new(observation, history);
        let mut usages = Vec::new();
        let mut sheds = 0;
        let started = Instant::now();
        for _ in 0..24 {
            let view = pages.view();
            let result = self
                .decide_view(&view, goal, &pages.history(), gate, &open)
                .await;
            match result {
                Err(error) if error.is::<context_budget::TooLarge>() => {
                    if pages.shrink() {
                        continue;
                    }
                    return Err(context_budget::with_usage(&usages, error));
                }
                Err(error)
                    if error.to_string().contains("max_tokens_exceeded")
                        && pages.retry_smaller() =>
                {
                    // This failed request has no reported usage. Preserve that
                    // uncertainty instead of counting it as a free decision.
                    usages.push(json!({}));
                    continue;
                }
                Err(error) => {
                    if let Some(usage) = JevBilled::usage_of(&error) {
                        usages.push(usage.clone());
                    }
                    // The service sheds large requests far more often than
                    // small ones: a large one is asked again smaller, while a
                    // small one still refused is an outage.
                    let large = context_budget::sheddable(&reports.lock().unwrap());
                    if Shed::is(&error) && large && pages.shed() {
                        sheds += 1;
                        continue;
                    }
                    return Err(context_budget::with_usage(&usages, error));
                }
                Ok(mut decision) => {
                    usages.push(decision.usage.clone());
                    if pages.advance(&decision.operation) {
                        continue;
                    }
                    decision.usage =
                        serde_json::to_value(crate::usage::JevCallUsage::sum(usages.iter()))?;
                    decision.usage["context_budget"] = json!({
                        "per_request_limit":context_budget::REQUEST_LIMIT,
                        "per_question_limit":context_budget::QUESTION_LIMIT,
                        "estimate":"UTF-8 byte upper estimate plus image patch reserve; provider is authoritative",
                        "requests":*reports.lock().unwrap(), "shed":sheds, "pages_read":pages.reads(), "context_reads":pages.read_history(),
                    });
                    decision.latency_ms = started.elapsed().as_millis() as u64;
                    return Ok(decision);
                }
            }
        }
        Err(context_budget::with_usage(
            &usages,
            crate::JevStop::new(
                crate::JevStatus::BudgetExceeded,
                "Reached Jev context-page read budget; no browser action executed",
            )
            .into(),
        ))
    }

    async fn decide_view(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
        gate: bool,
        transport: &dyn JevDecisionTransport,
    ) -> anyhow::Result<JevDecision> {
        let image = match observation["_screenshot"].as_str() {
            Some(screenshot) if self.images => images::fit(screenshot).ok(),
            _ => None,
        };
        let imaged = Imaged {
            inner: transport,
            image,
            observation,
            placement: if matches!(
                self.profile,
                crate::jev_prompt::Profile::DomTargets | crate::jev_prompt::Profile::Activation
            ) && observation["actions"].as_array().is_some_and(|actions| {
                actions.iter().any(|a| {
                    a["kind"] == "fill"
                        && a["value_clipped"] == true
                        && a["value"].as_str().is_some_and(|value| !value.is_empty())
                })
            }) {
                images::Placement::Targets
            } else {
                images::Placement::State
            },
        };
        choose(
            observation,
            goal,
            history,
            &self.model,
            &crate::jev_prompt::Transport {
                inner: &imaged,
                profile: self.profile,
                history,
                observation,
            },
            chrono::Local::now().date_naive(),
            DecisionOptions {
                gate,
                activation_audit: (self.profile == crate::jev_prompt::Profile::Activation)
                    .then_some(transport),
            },
        )
        .await
        .map(|(decision, _)| decision)
    }
}
