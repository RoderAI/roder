//! Paired live browser outcomes. Alternate provider order within each task.
use super::{Row, TaskValues, load_all, run_task, validate, write_rows};
use crate::{
    JevDecisionClient, JevTypeSafeDecisionClient, OpenAiDecisionsClient, fixture_harness::Harness,
};
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[tokio::test]
#[ignore = "paired live Decisions/Jev evaluation; requires both API keys and Chrome"]
async fn decisions_vs_jev() {
    let env = crate::runner::env_value;
    let reverse = match env("JEV_EVAL_CHOICE_ORDER").as_deref() {
        None | Some("original") => false,
        Some("reversed") => true,
        _ => panic!("JEV_EVAL_CHOICE_ORDER must be original or reversed"),
    };
    let openai = env("OPENAI_API_KEY")
        .or_else(|| roder_config::provider_api_key("openai"))
        .expect("OpenAI key required");
    let jev = env("JEV_API_KEY")
        .or_else(|| env("TYPESAFE_API_KEY"))
        .or_else(|| roder_config::provider_api_key("jev"))
        .expect("Jev key required");
    use crate::decisions::{DecisionsHttp, planning::Strategy};
    let wire = Arc::new(Recorded {
        inner: Arc::new(DecisionsHttp {
            key: openai,
            url: "https://api.openai.com/v1/decisions".into(),
            http: crate::http::JsonPoster::new(crate::http::RetryPolicy::default()),
        }),
        records: Arc::new(Mutex::new(Vec::new())),
        reverse,
    });
    let jev_wire = Arc::new(Recorded {
        inner: Arc::new(crate::decide::TypeSafeHttpTransport::new(
            crate::decide::ENDPOINT,
            jev,
            crate::http::RetryPolicy::default(),
        )),
        records: wire.records.clone(),
        reverse,
    });
    use crate::jev_prompt::Profile;
    let jev_variants = [
        ("jev", Profile::Baseline),
        ("jev_dom", Profile::Dom),
        ("jev_dom_targets", Profile::DomTargets),
        ("jev_activation", Profile::Activation),
        ("jev_literal", Profile::Literal),
        ("jev_effects", Profile::Effects),
        ("jev_criteria", Profile::Criteria),
        ("jev_available", Profile::Available),
        ("jev_joint", Profile::Joint),
        ("jev_evidence", Profile::Evidence),
        ("jev_scoped", Profile::Scoped),
        ("jev_form", Profile::Form),
        ("jev_focused", Profile::Focused),
        ("jev_workflow", Profile::Workflow),
    ];
    let variants = [
        ("original", Strategy::Original),
        ("refusals", Strategy::Refusals),
        ("native", Strategy::Native),
        ("joint", Strategy::Joint),
        ("sequential", Strategy::Sequential),
        ("verified", Strategy::Verified),
        ("effects", Strategy::Effects),
        ("vision", Strategy::Vision),
        ("vision_dom", Strategy::VisionDom),
        ("vision_activation", Strategy::VisionAudited),
        ("grounded", Strategy::Grounded),
    ];
    // A TypeSafe variant may pin its own model as `name@model` (for example
    // `jev@jev-1.13.0,jev@jev-preview`), so model versions run in one paired
    // dataset; without it, `JEV_MODEL` (default `jev-latest`) applies.
    // Append `-vision` to any Jev profile to add the viewport screenshot.
    // For example: `jev_workflow-vision@jev-preview`.
    let wanted = env("DECISIONS_EVAL_VARIANTS").unwrap_or_else(|| "vision,jev".into());
    let wanted_list = wanted.split(',').map(str::trim).collect::<Vec<_>>();
    for spec in &wanted_list {
        let (name, _) = parse_spec(spec).unwrap();
        assert!(
            jev_variants
                .iter()
                .any(|(n, _)| *n == name.strip_suffix("-vision").unwrap_or(name))
                || (variants.iter().any(|(n, _)| *n == name) && !spec.contains('@')),
            "Unknown variant {spec}"
        );
    }
    let mut seen = std::collections::HashSet::new();
    assert!(
        wanted_list.iter().all(|spec| seen.insert(*spec)),
        "Duplicate variant"
    );
    let mut providers: Vec<(&str, Arc<dyn JevDecisionClient>)> = variants
        .into_iter()
        .filter(|(name, _)| wanted_list.contains(name))
        .map(|(name, strategy)| {
            (
                name,
                Arc::new(OpenAiDecisionsClient::configured(wire.clone(), strategy))
                    as Arc<dyn JevDecisionClient>,
            )
        })
        .collect();
    for spec in &wanted_list {
        let (name, model) = parse_spec(spec).unwrap();
        let model = model
            .map(str::to_string)
            .unwrap_or_else(|| env("JEV_MODEL").unwrap_or_else(|| "jev-latest".into()));
        let images = name.ends_with("-vision");
        let profile_name = name.strip_suffix("-vision").unwrap_or(name);
        let Some((_, profile)) = jev_variants.iter().find(|(n, _)| *n == profile_name) else {
            continue;
        };
        let profile = *profile;
        providers.push((
            spec,
            Arc::new(
                JevTypeSafeDecisionClient::with_transport(model, jev_wire.clone())
                    .with_profile(profile)
                    .with_images(images),
            ),
        ));
    }
    let tasks = if let Some(suite) = env("BROWSER_EVAL_SUITE") {
        match suite.as_str() {
            "development" => {
                let mut tasks = load_all().unwrap();
                tasks.extend(super::complex_contracts::load(false));
                tasks
            }
            "complex_dev" => super::complex_contracts::load(false),
            "complex_holdout" => super::complex_contracts::load(true),
            "completion_dev" => super::completion_contracts::load(false),
            "completion_holdout" => super::completion_contracts::load(true),
            _ => panic!("Unknown BROWSER_EVAL_SUITE"),
        }
    } else if env("DECISIONS_EVAL_HOLDOUT").as_deref() == Some("1") {
        serde_json::from_str::<Vec<super::Task>>(include_str!(
            "../../../tests/fixtures/evals/decisions-holdout.json"
        ))
        .unwrap()
    } else {
        load_all().unwrap()
    };
    // Apply the same checked task filter to basic, compound and regression
    // suites, so focused model probes do not silently run the whole suite.
    let tasks = super::select(tasks, env("JEV_EVAL_TASKS").as_deref()).unwrap();
    validate(&tasks).unwrap();
    assert!(!tasks.is_empty());
    let repeats = env("JEV_EVAL_REPEATS")
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(3);
    assert!((1..=10).contains(&repeats));
    let fallback = super::fallback_live::live_fallback().await;
    let base = Harness::start().await.expect("Chrome required");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("decisions-vs-jev-{stamp}-{}", std::process::id());
    let mut rows = Vec::new();
    for repeat in 0..repeats {
        for (index, original) in tasks.iter().enumerate() {
            let mut task = original.clone();
            task.timeout_s = task.timeout_s.min(60);
            for slot in 0..providers.len() {
                let (provider, client) = &providers[(repeat + index + slot) % providers.len()];
                wire.records.lock().unwrap().clear();
                let harness = base.with_new_site().await;
                let outcome = run_task(
                    &harness,
                    &task,
                    client.clone(),
                    Arc::new(TaskValues::new(&task.values)),
                    task.expect.probes().cloned().collect(),
                    fallback.clone(),
                )
                .await;
                let mut row = match outcome {
                    Ok(outcome) => {
                        let mut row = Row::new(
                            &task,
                            "paired-live",
                            vec![provider.to_string()],
                            &outcome,
                            task.expect.grade(&outcome),
                        );
                        if fallback.is_some() {
                            row.fallback =
                                Some(super::FallbackRow::grade(&task, &outcome, row.marks));
                        }
                        row.telemetry = json!({"model":outcome.result.decisions.iter().rev().find_map(|d| d.model.clone()),
                            "usage":outcome.result.usage, "browser_decision_attempts":outcome.watch.laps.get("decide").map(|lap|lap.calls), "dom_probes":outcome.probed.dom, "decisions":outcome.result.decisions.iter().map(|d|json!({"operation":d.operation,"target":d.target,"confidence":d.confidence,"target_confidence":d.target_confidence,"probabilities":d.probabilities})).collect::<Vec<_>>(), "actions":outcome.result.actions,
                            "decision_latency_ms":outcome.result.decisions.iter().map(|d| d.latency_ms).collect::<Vec<_>>()});
                        row
                    }
                    Err(error) => Row::errored(&task, "paired-live", &error),
                };
                row.telemetry["wire"] = json!(*wire.records.lock().unwrap());
                row.telemetry["choice_order"] = json!(if reverse && provider.starts_with("jev") {
                    "reversed"
                } else {
                    "original"
                });
                row.telemetry["provider"] = json!(provider);
                row.repeat = repeat + 1;
                eprintln!(
                    "repeat={} provider={} task={} pass={} false_green={} ms={} calls={} failures={:?}",
                    repeat + 1,
                    provider,
                    row.task,
                    row.passed(),
                    row.marks.false_green,
                    row.wall_ms,
                    row.model_calls,
                    row.failures
                );
                rows.push(row);
                write_rows(&name, &rows).unwrap();
            }
        }
    }
    eprintln!("rows: {}", write_rows(&name, &rows).unwrap().display());
    assert_eq!(rows.len(), tasks.len() * repeats * providers.len());
}

struct Recorded {
    inner: Arc<dyn crate::JevDecisionTransport>,
    records: Arc<Mutex<Vec<serde_json::Value>>>,
    reverse: bool,
}
#[async_trait::async_trait]
impl crate::JevDecisionTransport for Recorded {
    async fn decide(&self, request: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let started = std::time::Instant::now();
        let mut request = request.clone();
        if self.reverse {
            reverse_choices(&mut request);
        }
        let result = self.inner.decide(&request).await;
        self.records.lock().unwrap().push(json!({"request":request,"response":result.as_ref().ok(),"latency_ms":started.elapsed().as_millis(),"error":result.as_ref().err().map(ToString::to_string)}));
        result
    }
}

#[tokio::test]
async fn decisions_holdout_fixture_contracts() {
    let tasks: Vec<super::Task> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/evals/decisions-holdout.json"
    ))
    .unwrap();
    validate(&tasks).unwrap();
    let base = harness_or_skip!();
    for task in tasks {
        let harness = base.with_new_site().await;
        let outcome = run_task(
            &harness,
            &task,
            Arc::new(super::scripted::StepDecider::new(&task.script.plan)),
            Arc::new(TaskValues::new(&task.values)),
            task.expect.probes().cloned().collect(),
            None,
        )
        .await
        .unwrap();
        let failures = task.expect.grade(&outcome).failures();
        assert!(failures.is_empty(), "{}: {failures:?}", task.id);
    }
}

/// TypeSafe only: reverse candidate order without changing ids or meanings.
fn reverse_choices(request: &mut serde_json::Value) {
    if let Some(questions) = request["questions"].as_object_mut() {
        for question in questions.values_mut().filter(|q| q["type"] == "choice") {
            if let Some(criteria) = question["criteria"].as_object_mut() {
                *criteria = std::mem::take(criteria).into_iter().rev().collect();
            }
        }
    }
}
#[test]
fn reversed_order_preserves_candidate_meaning_and_safety_questions() {
    let mut request = json!({"questions":{"operation":{"type":"choice","criteria":{"CLICK":"click","DONE":"finished","BLOCKED":"blocked"}},"gate":{"type":"noul","instructions":"Is this irreversible?"}}});
    let gate = request["questions"]["gate"].clone();
    reverse_choices(&mut request);
    assert_eq!(
        request["questions"]["operation"]["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["BLOCKED", "DONE", "CLICK"]
    );
    assert_eq!(
        request["questions"]["operation"]["criteria"]["DONE"],
        "finished"
    );
    assert_eq!(request["questions"]["gate"], gate);
}

/// Reject a malformed model pin before any provider can make a paid call.
fn parse_spec(spec: &str) -> anyhow::Result<(&str, Option<&str>)> {
    match spec.split_once('@') {
        Some((name, model)) => {
            let model = model.trim();
            anyhow::ensure!(!model.is_empty(), "Missing model in variant {spec}");
            Ok((name, Some(model)))
        }
        None => Ok((spec, None)),
    }
}

#[test]
fn model_pins_and_image_profiles_parse_before_provider_calls() {
    assert_eq!(
        parse_spec("jev_dom-vision@jev-preview").unwrap(),
        ("jev_dom-vision", Some("jev-preview"))
    );
    assert_eq!(
        parse_spec("jev_workflow-vision").unwrap(),
        ("jev_workflow-vision", None)
    );
    for spec in ["jev@", "jev-vision@   "] {
        assert!(parse_spec(spec).is_err(), "{spec}");
    }
}
