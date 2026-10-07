use super::*;

#[tokio::test]
async fn full_access_suppresses_ordinary_review_but_retains_explicit_denials() {
    for contribution in [
        PolicyContribution::RequireApproval {
            reason: Some("review".into()),
        },
        PolicyContribution::Deny {
            reason: "host restriction".into(),
        },
    ] {
        let decision = DefaultPolicyGate::new()
            .decide_with_contributors(
                &tool_call("shell", json!({"command": "cargo test"})),
                PolicyMode::Bypass,
                &context(PolicyMode::Bypass),
                &[Arc::new(StaticPolicyContributor {
                    id: "review",
                    contribution: contribution.clone(),
                })],
            )
            .await
            .unwrap();
        match contribution {
            PolicyContribution::Deny { .. } => {
                assert!(matches!(decision, PolicyDecision::Denied { .. }))
            }
            _ => assert!(matches!(decision, PolicyDecision::AutoApproved { .. })),
        }
    }
}

struct SwitchToFullAccess {
    runtime: Mutex<Option<std::sync::Weak<Runtime>>>,
}

#[async_trait::async_trait]
impl PolicyContributor for SwitchToFullAccess {
    fn id(&self) -> String {
        "switch-full-access".into()
    }
    async fn review_tool(&self, review: PolicyReview) -> anyhow::Result<PolicyContribution> {
        assert_eq!(
            review
                .context
                .handles
                .workspace
                .as_ref()
                .unwrap()
                .workspace_root(),
            Some(std::env::current_dir().unwrap()),
            "permission switching must preserve the original tool context"
        );
        let runtime = self
            .runtime
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        if review.mode == PolicyMode::Default {
            runtime
                .set_policy_mode(PolicyMode::Bypass, Some("switch during review".into()))
                .await?;
        }
        Ok(PolicyContribution::RequireApproval {
            reason: Some("ordinary review".into()),
        })
    }
}

#[tokio::test]
async fn full_access_switch_before_approval_registration_does_not_strand_turn() {
    let contributor = Arc::new(SwitchToFullAccess {
        runtime: Mutex::new(None),
    });
    let seen = Arc::new(Mutex::new(vec![]));
    let runtime = runtime_with_policy_contributor(
        PolicyMode::Default,
        "shell",
        json!({"command": "cargo test"}),
        seen.clone(),
        contributor.clone(),
    );
    *contributor.runtime.lock().unwrap() = Some(Arc::downgrade(&runtime));
    let mut events = runtime.subscribe_events();
    runtime
        .start_turn(StartTurnRequest {
            thread_id: "goal-permission-switch".into(),
            message: "Run the authorized checks".into(),
            images: vec![],
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.unwrap().event {
                RoderEvent::ApprovalRequested(_) => panic!("Full Access must not request approval"),
                RoderEvent::TurnCompleted(_) => break,
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(*seen.lock().unwrap(), [PolicyMode::Bypass]);
}
