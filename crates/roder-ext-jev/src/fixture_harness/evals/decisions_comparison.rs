//! Paired live browser outcomes. Alternate provider order within each task.
use super::{Row, TaskValues, load_tasks, run_task, validate, write_rows};
use crate::{
    JevDecisionClient, JevTypeSafeDecisionClient, OpenAiDecisionsClient, fixture_harness::Harness,
};
use serde_json::json;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[tokio::test]
#[ignore = "paired live Decisions/Jev evaluation; requires both API keys and Chrome"]
async fn decisions_vs_jev() {
    let env = crate::runner::env_value;
    let openai = env("OPENAI_API_KEY")
        .or_else(|| roder_config::provider_api_key("openai"))
        .expect("OpenAI key required");
    let jev = env("JEV_API_KEY")
        .or_else(|| env("TYPESAFE_API_KEY"))
        .or_else(|| roder_config::provider_api_key("jev"))
        .expect("Jev key required");
    let providers: [(&str, Arc<dyn JevDecisionClient>); 2] = [
        ("decisions", Arc::new(OpenAiDecisionsClient::new(openai))),
        (
            "jev",
            Arc::new(JevTypeSafeDecisionClient::new(
                jev,
                env("JEV_MODEL").unwrap_or_else(|| "jev-latest".into()),
            )),
        ),
    ];
    let tasks = load_tasks().unwrap();
    validate(&tasks).unwrap();
    assert!(!tasks.is_empty());
    let repeats = env("JEV_EVAL_REPEATS")
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(3);
    assert!((1..=10).contains(&repeats));
    let base = Harness::start().await.expect("Chrome required");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let name = format!("decisions-vs-jev-{stamp}");
    let mut rows = Vec::new();
    for repeat in 0..repeats {
        for (index, original) in tasks.iter().enumerate() {
            let mut task = original.clone();
            task.timeout_s = task.timeout_s.min(60);
            for slot in 0..2 {
                let (provider, client) = &providers[(repeat + index + slot) % 2];
                let harness = base.with_new_site().await;
                let outcome = run_task(
                    &harness,
                    &task,
                    client.clone(),
                    Arc::new(TaskValues::new(&task.values)),
                    task.expect.probes().cloned().collect(),
                    None,
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
                        row.telemetry = json!({"model":outcome.result.decisions.iter().rev().find_map(|d| d.model.clone()),
                            "usage":outcome.result.usage,
                            "decision_latency_ms":outcome.result.decisions.iter().map(|d| d.latency_ms).collect::<Vec<_>>()});
                        row
                    }
                    Err(error) => Row::errored(&task, "paired-live", &error),
                };
                row.telemetry["provider"] = json!(provider);
                row.telemetry["repeat"] = json!(repeat + 1);
                eprintln!(
                    "repeat={} provider={} task={} pass={} ms={} calls={} failures={:?}",
                    repeat + 1,
                    provider,
                    row.task,
                    row.pass,
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
    assert_eq!(rows.len(), tasks.len() * repeats * 2);
}
