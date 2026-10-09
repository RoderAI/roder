//! Strict live Decisions eval: missing credentials, Chrome or outcomes fail.
use super::{Row, TaskValues, load_tasks, run_task, table, validate, write_rows};
use crate::{OpenAiDecisionsClient, fixture_harness::Harness};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[tokio::test]
#[ignore = "live OpenAI Decisions calls; needs an OpenAI API key and Chrome"]
async fn decisions_live_browser_corpus() {
    let key = crate::runner::env_value("OPENAI_API_KEY")
        .or_else(|| roder_config::provider_api_key("openai"))
        .expect("live Decisions validation requires an OpenAI API key");
    let tasks = load_tasks().unwrap();
    validate(&tasks).unwrap();
    assert!(!tasks.is_empty(), "live validation must execute tasks");
    let base = Harness::start()
        .await
        .expect("live validation requires Chrome");
    let decision = Arc::new(OpenAiDecisionsClient::new(key));
    let mut rows = Vec::new();
    for task in &tasks {
        let harness = base.with_new_site().await;
        let outcome = run_task(
            &harness,
            task,
            decision.clone(),
            Arc::new(TaskValues::new(&task.values)),
            task.expect.probes().cloned().collect(),
            None,
        )
        .await;
        let row = match outcome {
            Ok(outcome) => Row::new(
                task,
                "decisions-live",
                Vec::new(),
                &outcome,
                task.expect.grade(&outcome),
            ),
            Err(error) => Row::errored(task, "decisions-live", &error),
        };
        rows.push(row);
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let path = write_rows(&format!("decisions-live-{stamp}"), &rows).unwrap();
    eprintln!("{}rows: {}", table(&tasks, &rows), path.display());
    assert!(
        rows.iter().all(|row| row.passed() && row.model_calls > 0),
        "Decisions live outcomes failed; inspect {}",
        path.display()
    );
}
