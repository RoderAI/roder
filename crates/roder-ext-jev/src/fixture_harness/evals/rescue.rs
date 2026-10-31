//! Measure a one-tool Terra rescue followed by the same native engine, with
//! unchanged budgets, rules and graders. This experiment does not change defaults.
use super::*;
use crate::{JevDecisionClient, OpenAiDecisionsClient};
use serde_json::json;

#[tokio::test]
#[ignore = "live bounded rescue experiment; both API keys, Codex sign-in and Chrome"]
async fn bounded_rescue_handback() {
    let env = crate::runner::env_value;
    let jev = super::live::live_route().expect("Jev decision key");
    let openai = env("OPENAI_API_KEY").expect("OpenAI key");
    let model = fallback_live::live_fallback()
        .await
        .expect("set JEV_EVAL_FALLBACK=model");
    let clients: Vec<(&str, Arc<dyn JevDecisionClient>)> = vec![
        (
            "decisions",
            Arc::new(
                OpenAiDecisionsClient::new(openai)
                    .with_browser_profile(crate::DecisionsBrowserProfile::Audited),
            ),
        ),
        (
            "jev",
            Arc::new(
                jev.client("jev-latest")
                    .with_profile(crate::jev_prompt::Profile::Activation)
                    .with_images(jev.host.takes_images()),
            ),
        ),
    ];
    let tasks = load_tasks().unwrap();
    let base = Harness::start().await.expect("Chrome");
    let mut rows = Vec::new();
    for task in &tasks {
        for (driver, client) in &clients {
            let harness = base.with_new_site().await;
            let page = harness.open(&task.page).await.unwrap();
            let target = page.target_id().to_string();
            let watch = Watch::default();
            let (browser, probed) =
                ProbedPage::new(page, task.expect.probes().cloned().collect(), watch.clone());
            let mut config = JevEngineConfig::new(&task.goal, watch.decisions(client.clone()))
                .with_text_resolver(Arc::new(TaskValues::new(&task.values)))
                .with_scope(super::config_scope(&harness, task).unwrap())
                .with_max_actions(12)
                .with_wait(Duration::from_millis(100));
            if task.confirm_irreversible {
                config = config.with_irreversible_gate();
            }
            if task.authorize_irreversible {
                config = config.with_irreversible_authorized();
            }
            let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
            let started = Instant::now();
            let native = engine.run(Duration::from_secs(task.timeout_s)).await;
            let mut final_native = native.clone();
            let mut rescue = None;
            let mut resumed = false;
            if let Some(trigger) = crate::fallback::trigger::trigger(&native) {
                let tab = roder_ext_chrome::direct::DirectTab::Target {
                    endpoint: harness.endpoint().into(),
                    target_id: target.clone(),
                };
                let rules = Rules {
                    scope: super::config_scope(&harness, task).unwrap(),
                    gate: task.confirm_irreversible,
                    authorized: task.authorize_irreversible,
                    banners: task.refuse_cookie_banners.unwrap_or(true),
                    secrets: native.typed_secrets.clone(),
                };
                let goal = format!(
                    "Resolve the immediate browser obstacle, then leave the page ready for the native driver to continue this task: {}",
                    task.goal
                );
                let (outcome, secrets) = crate::fallback::fall_back(
                    &tab,
                    model.as_ref(),
                    crate::fallback::Brief {
                        goal: &goal,
                        trigger,
                        jev: &native,
                    },
                    rules,
                    Limits {
                        max_steps: 1,
                        max_tokens: 100_000,
                        deadline: tokio::time::Instant::now() + Duration::from_secs(40),
                    },
                    None,
                )
                .await;
                // Never hand back a rule refusal, uncertain side effect, or another tab.
                if outcome.status == crate::JevStatus::BudgetExceeded
                    && outcome.target_id == target
                    && !outcome.actions.is_empty()
                    && outcome.actions.iter().all(|a| !a.error)
                {
                    if let Some(page) = &outcome.last_page
                        && engine.resume_after_rescue(page, &secrets).await.is_ok()
                    {
                        resumed = true;
                        final_native = engine
                            .run(
                                Duration::from_secs(task.timeout_s)
                                    .saturating_sub(started.elapsed()),
                            )
                            .await;
                    }
                }
                rescue = Some(outcome);
            }
            engine.close().await.ok();
            let outcome = Outcome {
                result: final_native.clone(),
                probed: probed.take(),
                posts: harness
                    .site
                    .wait_for_posts(
                        task.expect.posts.as_ref().map_or(0, Vec::len),
                        Duration::from_secs(1),
                    )
                    .await,
                wall_ms: started.elapsed().as_millis() as u64,
                watch: watch.snapshot(final_native.elapsed_ms),
                after: None,
            };
            let expected = task.fallback.as_ref().map_or(&task.expect, |f| &f.expect);
            let grade = expected.grade(&outcome);
            rows.push(json!({"task":task.id,"driver":driver,"initial":native,"rescue":rescue,"resumed":resumed,"final":final_native,"wall_ms":outcome.wall_ms,"grade":grade.marks(),"failures":grade.failures()}));
            eprintln!(
                "rescue driver={driver} task={} resumed={resumed} status={:?}",
                task.id, final_native.status
            );
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let path = write_json(&format!("rescue-handback-{stamp}.json"), &json!(rows)).unwrap();
    eprintln!("rescue rows: {}", path.display());
}
