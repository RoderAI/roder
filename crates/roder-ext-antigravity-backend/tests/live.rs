//! Explicit opt-in tests: these use the user's real subscription.
use roder_api::backend::{AgentBackend, BackendEvent, BackendStart};
use roder_api::policy_mode::PolicyMode;
use roder_ext_antigravity_backend::AntigravityBackend;
use std::time::Duration;

#[tokio::test]
#[ignore = "uses authenticated Antigravity subscription; set RODER_LIVE_ANTIGRAVITY=1"]
async fn subscription_prompt_and_cleanup() -> anyhow::Result<()> {
    anyhow::ensure!(
        std::env::var("RODER_LIVE_ANTIGRAVITY").as_deref() == Ok("1"),
        "Set RODER_LIVE_ANTIGRAVITY=1 to opt into subscription inference"
    );
    let cwd = std::env::current_dir()?;
    let backend = AntigravityBackend::default();
    let models = backend.list_models(cwd.clone()).await?;
    anyhow::ensure!(!models.is_empty(), "ACP must discover account models");
    println!("Discovered {} account models", models.len());
    let mut session = backend
        .connect(BackendStart {
            cwd,
            model: std::env::var("RODER_LIVE_ANTIGRAVITY_MODEL").ok(),
            resume_thread: None,
            policy_mode: PolicyMode::Bypass,
        })
        .await?;
    println!("Selected account model: {}", session.model());
    session.send(
        "Do not use tools, read files, or modify anything. Reply with exactly RODER_ANTIGRAVITY_OK."
            .into(), None, None, None,
    ).await?;
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            match session.next_event().await? {
                BackendEvent::Text(chunk) => text.push_str(&chunk),
                BackendEvent::Approval { request_id, .. } => {
                    session.approve(&request_id, false).await?
                }
                BackendEvent::Question { request_id, .. } => {
                    session.answer(&request_id, vec![]).await?
                }
                BackendEvent::TurnFinished { status, .. } => {
                    anyhow::ensure!(status == "completed", "Live turn failed: {status}");
                    break;
                }
                _ => {}
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    anyhow::ensure!(
        text.contains("RODER_ANTIGRAVITY_OK"),
        "Expected response marker, got {text:?}"
    );
    session
        .send(
            "Do not use tools. Count slowly from 1 to 1000.".into(),
            None,
            None,
            None,
        )
        .await?;
    session.interrupt().await?;
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            match session.next_event().await? {
                BackendEvent::TurnFinished { status, .. } => {
                    anyhow::ensure!(
                        status == "interrupted" || status == "completed",
                        "Cancellation failed: {status}"
                    );
                    break;
                }
                BackendEvent::Approval { request_id, .. } => {
                    session.approve(&request_id, false).await?
                }
                _ => {}
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    session.archive().await?;
    println!("Live prompt, cancellation, and process cleanup passed");
    Ok(())
}
