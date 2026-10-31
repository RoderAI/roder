//! The consult and the light recovery tier on real pages: an unsure repeat
//! the consultant endorses loads the rest of a feed, an unsure step acts on
//! the control the consultant names (still gated), and a stall gets a
//! bounded one-step recovery before it ends `blocked`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use super::Harness;
use super::scripted::{PlanDecider, pick};
use crate::engine::{
    JevConsultAnswer, JevConsultant, JevDecision, JevDecisionClient, JevEngine, JevEngineConfig,
    JevRunResult, JevStatus, JevStopCause,
};

const RUN: Duration = Duration::from_secs(30);

/// A scripted plan whose decisions carry these call confidences, by step.
struct Unsure {
    plan: PlanDecider,
    confidence: Vec<f64>,
}

#[async_trait]
impl JevDecisionClient for Unsure {
    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        let mut decision = self.plan.choose(observation, goal, history).await?;
        if let Some(confidence) = self.confidence.get(history.len()) {
            decision.confidence = *confidence;
        }
        Ok(decision)
    }
}

fn unsure(plan: Vec<super::scripted::Pick>, confidence: &[f64]) -> Arc<Unsure> {
    Arc::new(Unsure {
        plan: PlanDecider::new(plan),
        confidence: confidence.to_vec(),
    })
}

/// Names the listed control with this label, or none; keeps each request.
struct Names {
    label: Option<&'static str>,
    asked: Mutex<Vec<Value>>,
}

impl Names {
    fn new(label: Option<&'static str>) -> Arc<Self> {
        Arc::new(Self {
            label,
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<Value> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl JevConsultant for Names {
    async fn consult(&self, request: &Value) -> anyhow::Result<JevConsultAnswer> {
        self.asked.lock().unwrap().push(request.clone());
        let control = self.label.and_then(|label| {
            request["controls"]
                .as_array()?
                .iter()
                .find(|control| control["label"] == label)
                .and_then(|control| control["id"].as_str().map(str::to_string))
        });
        Ok(JevConsultAnswer {
            control,
            diagnosis: "scripted".into(),
            model: "scripted-consultant".into(),
            latency_ms: 0,
            usage: serde_json::json!({"input_tokens": 100, "output_tokens": 10}),
        })
    }
}

async fn run(
    harness: &Harness,
    page: &str,
    decider: Arc<dyn JevDecisionClient>,
    consultant: Option<Arc<Names>>,
    configure: impl FnOnce(JevEngineConfig) -> JevEngineConfig,
) -> JevRunResult {
    let page = harness.open(page).await.expect("open the fixture page");
    let mut config = JevEngineConfig::new("scripted fixture goal", decider)
        .with_wait(Duration::from_millis(100))
        .with_cookie_banner_refusal(false);
    if let Some(consultant) = consultant {
        // Both tiers on, as `JEV_CONSULT_BELOW=0.55 JEV_LIGHT_RECOVERIES=2`
        // turn them on; a test may set either again.
        config = config
            .with_consultant(consultant)
            .with_consult_below(0.55)
            .with_light_recoveries(2);
    }
    let mut engine = JevEngine::start(Box::new(page), configure(config))
        .await
        .expect("first observation");
    let result = engine.run(RUN).await;
    engine.close().await.ok();
    result
}

fn kinds(result: &JevRunResult) -> Vec<String> {
    result
        .recoveries
        .iter()
        .filter_map(|event| event["kind"].as_str().map(str::to_string))
        .collect()
}

fn labels(result: &JevRunResult) -> Vec<&str> {
    result.actions.iter().map(|a| a.action.as_str()).collect()
}

#[tokio::test]
async fn an_unsure_repeat_the_consultant_endorses_loads_the_rest_of_the_feed() {
    let harness = harness_or_skip!();
    let plan = || vec![pick("click", "Load more"), pick("click", "Load more")];

    // Without a consultant the guard stops the second, unsure click: the
    // false stop the live corpus saw on `mock_infinite_scroll`.
    let alone = run(
        &harness,
        "feed.html",
        unsure(plan(), &[0.9, 0.66]),
        None,
        |c| c,
    )
    .await;
    assert_eq!(alone.status, JevStatus::Blocked, "{alone:#?}");
    assert_eq!(alone.stop_cause, Some(JevStopCause::UncertainRepeat));

    let consultant = Names::new(Some("Load more"));
    let result = run(
        &harness,
        "feed.html",
        unsure(plan(), &[0.9, 0.66]),
        Some(consultant.clone()),
        |c| c,
    )
    .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert!(result.visible_text.contains("End of list. 60 items."));
    assert_eq!(labels(&result), ["Load more", "Load more"]);
    // One consult, about the repeat; a text call, not a step or a decision.
    let asked = consultant.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["purpose"], "consult");
    assert!(asked[0]["problem"].as_str().unwrap().contains("repeats"));
    assert_eq!(asked[0]["jev_proposal"]["label"], "Load more");
    assert_eq!(result.text_calls, 1);
    assert_eq!(result.usage.text.calls, 1);
    assert_eq!(
        serde_json::to_value(result.usage.text.input_tokens).unwrap(),
        100
    );
    assert_eq!(result.model_calls, 3);
    assert!(kinds(&result).contains(&"consult_endorsed".to_string()));
}

#[tokio::test]
async fn an_unsure_step_acts_on_the_control_the_consultant_names() {
    let harness = harness_or_skip!();
    let consultant = Names::new(Some("Continue"));
    let result = run(
        &harness,
        "stall.html",
        unsure(vec![pick("click", "Refresh status")], &[0.3]),
        Some(consultant.clone()),
        |c| c,
    )
    .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(labels(&result), ["Continue"]);
    assert!(result.visible_text.contains("Continued 1."));
    assert!(kinds(&result).contains(&"consult_swapped".to_string()));
    assert_eq!(consultant.asked()[0]["jev_proposal"]["confidence"], 0.3);

    // A step Jev is sure of is not asked about.
    let consultant = Names::new(Some("Continue"));
    let result = run(
        &harness,
        "stall.html",
        unsure(vec![pick("click", "Refresh status")], &[0.9]),
        Some(consultant.clone()),
        |c| c,
    )
    .await;
    assert_eq!(labels(&result), ["Refresh status"]);
    assert!(consultant.asked().is_empty());
    assert_eq!(result.text_calls, 0);

    // Nor is one under a threshold set to zero.
    let consultant = Names::new(Some("Continue"));
    let result = run(
        &harness,
        "stall.html",
        unsure(vec![pick("click", "Refresh status")], &[0.3]),
        Some(consultant.clone()),
        |c| c.with_consult_below(0.0),
    )
    .await;
    assert_eq!(labels(&result), ["Refresh status"]);
    assert!(consultant.asked().is_empty());
}

#[tokio::test]
async fn a_control_the_consultant_swaps_in_still_meets_the_irreversible_gate() {
    let harness = harness_or_skip!();
    let consultant = Names::new(Some("Delete application"));
    let result = run(
        &harness,
        "stall.html",
        unsure(vec![pick("click", "Refresh status")], &[0.3]),
        Some(consultant),
        |c| c.with_irreversible_gate().with_irreversible_authorized(),
    )
    .await;
    // No answer to the gate's question and no confidence: it stops even
    // with irreversible actions authorized.
    assert_eq!(result.status, JevStatus::NeedsConfirmation, "{result:#?}");
    assert!(result.actions.is_empty(), "{result:#?}");
    assert!(!result.visible_text.contains("Deleted."));
}

#[tokio::test]
async fn an_endorsed_repeat_of_a_commitment_still_stops() {
    let harness = harness_or_skip!();
    let plan = || {
        vec![
            pick("click", "Delete application"),
            pick("click", "Delete application"),
        ]
    };
    // Unsure enough for the guard but not for a consult: the repeat of a
    // commitment is not asked about at all.
    let consultant = Names::new(Some("Delete application"));
    let result = run(
        &harness,
        "stall.html",
        unsure(plan(), &[1.0, 0.6]),
        Some(consultant.clone()),
        |c| c,
    )
    .await;
    assert_eq!(result.stop_cause, Some(JevStopCause::UncertainRepeat));
    assert!(consultant.asked().is_empty());
    // Unsure enough for a consult, which endorses it: still stopped. So is
    // a second "Add to cart", which names no commitment word but adds one.
    for label in ["Delete application", "Add to cart"] {
        let plan = vec![pick("click", label), pick("click", label)];
        let consultant = Names::new(Some(label));
        let result = run(
            &harness,
            "stall.html",
            unsure(plan, &[1.0, 0.3]),
            Some(consultant.clone()),
            |c| c,
        )
        .await;
        assert_eq!(
            result.stop_cause,
            Some(JevStopCause::UncertainRepeat),
            "{label}"
        );
        assert_eq!(consultant.asked().len(), 1, "{label}");
        assert_eq!(labels(&result), [label]);
    }
}

#[tokio::test]
async fn a_stall_gets_a_light_recovery_and_jev_resumes() {
    let harness = harness_or_skip!();
    let plan = (0..3).map(|_| pick("click", "Refresh status")).collect();
    let consultant = Names::new(Some("Continue"));
    let result = run(
        &harness,
        "stall.html",
        Arc::new(PlanDecider::new(plan)),
        Some(consultant.clone()),
        |c| c,
    )
    .await;
    // Three that changed nothing, the recovery's step, then Jev's DONE.
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(
        labels(&result),
        [
            "Refresh status",
            "Refresh status",
            "Refresh status",
            "Continue"
        ]
    );
    assert!(result.visible_text.contains("Continued 1."));
    let asked = consultant.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["purpose"], "recover");
    assert_eq!(asked[0]["recent_steps"].as_array().unwrap().len(), 3);
    assert!(kinds(&result).contains(&"light_recovery".to_string()));
    // The recovery is a step, not a decision: four calls for four steps
    // and the DONE.
    assert_eq!(result.model_calls, 4);
}

#[tokio::test]
async fn a_recovery_that_names_nothing_or_what_failed_leaves_the_stall() {
    let harness = harness_or_skip!();
    for label in [None, Some("Refresh status")] {
        let plan = (0..3).map(|_| pick("click", "Refresh status")).collect();
        let consultant = Names::new(label);
        let result = run(
            &harness,
            "stall.html",
            Arc::new(PlanDecider::new(plan)),
            Some(consultant.clone()),
            |c| c,
        )
        .await;
        assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
        assert_eq!(result.stop_cause, Some(JevStopCause::Stalled));
        assert_eq!(result.actions.len(), 3);
        assert_eq!(consultant.asked().len(), 1);
        assert!(kinds(&result).contains(&"light_recovery_declined".to_string()));
        // It fell back as a stall always has.
        assert_eq!(
            crate::fallback::trigger::trigger(&result),
            Some(crate::fallback::trigger::Trigger::Stalled)
        );
    }
}

#[tokio::test]
async fn light_recoveries_are_bounded() {
    let harness = harness_or_skip!();
    // Jev keeps pressing what does nothing; each recovery's step helps for
    // a moment and the stall comes back.
    let plan = || (0..11).map(|_| pick("click", "Refresh status")).collect();
    let consultant = Names::new(Some("Continue"));
    let result = run(
        &harness,
        "stall.html",
        Arc::new(PlanDecider::new(plan())),
        Some(consultant.clone()),
        |c| c,
    )
    .await;
    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stop_cause, Some(JevStopCause::Stalled));
    assert_eq!(consultant.asked().len(), 2);
    assert_eq!(result.actions.len(), 11);
    assert!(result.visible_text.contains("Continued 2."));
    let recovered = kinds(&result)
        .iter()
        .filter(|kind| *kind == "light_recovery")
        .count();
    assert_eq!(recovered, 2);

    // None at all when the tier is set to zero.
    let consultant = Names::new(Some("Continue"));
    let result = run(
        &harness,
        "stall.html",
        Arc::new(PlanDecider::new(plan())),
        Some(consultant.clone()),
        |c| c.with_light_recoveries(0),
    )
    .await;
    assert_eq!(result.stop_cause, Some(JevStopCause::Stalled));
    assert_eq!(result.actions.len(), 3);
    assert!(consultant.asked().is_empty());
}

#[tokio::test]
async fn going_round_in_circles_asks_for_a_light_recovery_too() {
    let harness = harness_or_skip!();
    let plan = (0..60)
        .map(|n| pick("click", ["Open menu", "Close menu"][n % 2]))
        .collect();
    let consultant = Names::new(Some("Open menu"));
    let result = run(
        &harness,
        "toggle-menu.html",
        Arc::new(PlanDecider::new(plan)),
        Some(consultant.clone()),
        |c| c,
    )
    .await;
    // The only other way on is the control going round, which is no
    // recovery: the loop's stop stands, after one ask.
    assert_eq!(result.stop_cause, Some(JevStopCause::Looped), "{result:#?}");
    assert_eq!(result.actions.len(), 6);
    let asked = consultant.asked();
    assert_eq!(asked.len(), 1);
    assert!(asked[0]["problem"].as_str().unwrap().contains("circles"));
    assert!(kinds(&result).contains(&"light_recovery_declined".to_string()));
}

#[tokio::test]
async fn by_default_only_an_unsure_repeat_is_asked_about() {
    let harness = harness_or_skip!();
    let defaults = |consultant: Arc<Names>| move |c: JevEngineConfig| c.with_consultant(consultant);
    // An unsure step that repeats nothing goes as Jev chose it.
    let consultant = Names::new(Some("Continue"));
    let result = run(
        &harness,
        "stall.html",
        unsure(vec![pick("click", "Refresh status")], &[0.3]),
        None,
        defaults(consultant.clone()),
    )
    .await;
    assert_eq!(labels(&result), ["Refresh status"]);
    // A stall ends as it always has.
    let plan = (0..3).map(|_| pick("click", "Refresh status")).collect();
    let result = run(
        &harness,
        "stall.html",
        Arc::new(PlanDecider::new(plan)),
        None,
        defaults(consultant.clone()),
    )
    .await;
    assert_eq!(result.stop_cause, Some(JevStopCause::Stalled));
    assert!(consultant.asked().is_empty());
    // The unsure repeat is asked about, and endorsed it loads the feed.
    let consultant = Names::new(Some("Load more"));
    let plan = vec![pick("click", "Load more"), pick("click", "Load more")];
    let result = run(
        &harness,
        "feed.html",
        unsure(plan, &[0.9, 0.66]),
        None,
        defaults(consultant.clone()),
    )
    .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(consultant.asked().len(), 1);
}
