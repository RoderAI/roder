//! Live deployment authority enforced without a responsive executor client.
use super::*;
use futures::future::BoxFuture;

#[derive(Default)]
struct RevocablePolicy {
    denied: AtomicBool,
    hanging: AtomicBool,
}

impl HostedRequestPolicy for RevocablePolicy {
    fn evaluate(
        &self,
        _: &HostedRequestContext,
        _: &str,
        request: JsonRpcRequest,
    ) -> HostedRequestPolicyDecision {
        HostedRequestPolicyDecision::allow(request)
    }

    fn revalidate<'a>(
        &'a self,
        _: &'a HostedRequestContext,
        bearer: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if bearer != "rk_test_tenant_a_writer" {
                return Ok(());
            }
            if self.hanging.load(Ordering::Acquire) {
                futures::future::pending::<()>().await;
            }
            if self.denied.load(Ordering::Acquire) {
                return Err(format!("live denial {bearer}"));
            }
            Ok(())
        })
    }
}

async fn observer(fixture: &Fixture) -> Socket {
    fixture
        .authenticator
        .register_static_key(
            "rk_test_observer",
            PrincipalSeed {
                tenant_id: "tenant-a".into(),
                principal: PrincipalContext::User {
                    user_id: "user-tenant-a".into(),
                    display_name: None,
                },
                role: HostedRole::TenantAdmin,
                scopes: vec![HostedScope::Read, HostedScope::Write, HostedScope::Admin],
            },
        )
        .unwrap();
    connect(&fixture.url, "rk_test_observer").await.unwrap()
}

#[tokio::test]
async fn idle_revocation_terminalizes_pending_executor_without_client_requests() {
    let policy = Arc::new(RevocablePolicy::default());
    let fixture = fixture_with_policy(
        "live-policy",
        RateLimitConfig::default(),
        true,
        policy.clone(),
    )
    .await;
    let mut owner = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let mut ordinary = observer(&fixture).await;
    let directory = temp_dir("live-policy-workspace");
    let workspace = call(
        &mut owner,
        "workspace/create",
        serde_json::json!({
            "roots":[{"path":directory}],"defaultRootPath":directory
        }),
    )
    .await
    .result
    .unwrap()["workspace"]
        .clone();
    let thread = call(&mut owner, "thread/start", serde_json::json!({
        "workspaceId":workspace["id"],"model":"mock",
        "externalTools":[{"name":"acme_lookup","description":"lookup","parameters":{"type":"object"}}]
    })).await.result.unwrap()["thread"]["id"].clone();
    let old_lease = call(
        &mut owner,
        "tools/bind_executor",
        serde_json::json!({"threadId":thread}),
    )
    .await
    .result
    .unwrap()["executor"]
        .clone();
    call(
        &mut owner,
        "turn/start",
        serde_json::json!({"threadId":thread,"prompt":"FAKE_EXTERNAL_TOOL lookup"}),
    )
    .await
    .result
    .unwrap();
    let pending = hosted_executor::notification(&mut owner, "thread/toolExecutionRequested").await;
    policy.denied.store(true, Ordering::Release);
    // Do not send a resolve, unbind, ping, or any other owner request. The
    // server's idle check must revoke this still-connected executor itself.
    let revoked = hosted_executor::notification(&mut ordinary, "tools/executorRevoked").await;
    assert_eq!(revoked["executor"], old_lease);
    assert_eq!(revoked["reason"], "authorization_revoked");
    let fresh_lease = call(
        &mut ordinary,
        "tools/bind_executor",
        serde_json::json!({"threadId":thread}),
    )
    .await
    .result
    .unwrap()["executor"]
        .clone();
    assert_ne!(fresh_lease["leaseId"], old_lease["leaseId"]);
    let state = call(
        &mut ordinary,
        "tools/execution_read",
        serde_json::json!({"executor":fresh_lease,"requestId":pending["requestId"]}),
    )
    .await
    .result
    .unwrap();
    assert_eq!(state["execution"]["state"], "cancelled");
    assert!(call(&mut ordinary, "tools/resolve", serde_json::json!({
        "executor":old_lease,"turnId":pending["turnId"],"requestId":pending["requestId"],"output":"late success"
    })).await.error.is_some());
    let server = fixture.pool.lease("tenant-a").await.unwrap().server.clone();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while server.runtime.active_turn_count().await != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        call(&mut ordinary, "hosted/whoami", serde_json::json!({}))
            .await
            .error
            .is_none()
    );
    let records = fixture.audit.for_tenant("tenant-a");
    assert!(
        records
            .iter()
            .any(|record| record.kind == "auth_revalidation_failed"
                && record.reason.as_deref() == Some("live denial [REDACTED]"))
    );
    policy.denied.store(false, Ordering::Release);
    let mut reconnect = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let history = call(
        &mut reconnect,
        "thread/read",
        serde_json::json!({"threadId":thread,"includeTurns":true}),
    )
    .await
    .result
    .unwrap();
    assert_ne!(
        history["thread"]["turns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|turn| turn["id"] == pending["turnId"])
            .unwrap()["status"],
        "inProgress"
    );
    assert!(call(&mut reconnect, "tools/resolve", serde_json::json!({
        "executor":old_lease,"turnId":pending["turnId"],"requestId":pending["requestId"],"output":"replay"
    })).await.error.is_some());
    ordinary.close(None).await.unwrap();
    reconnect.close(None).await.unwrap();
    fixture.controller.stop().await.unwrap();
}

#[tokio::test]
async fn live_policy_denies_before_dispatch_and_admission_does_not_start_runtime() {
    let policy = Arc::new(RevocablePolicy::default());
    let fixture = fixture_with_policy(
        "live-dispatch",
        RateLimitConfig::default(),
        true,
        policy.clone(),
    )
    .await;
    let mut owner = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    assert!(
        call(&mut owner, "hosted/whoami", serde_json::json!({}))
            .await
            .error
            .is_none()
    );
    policy.denied.store(true, Ordering::Release);
    let response = call(&mut owner, "thread/start", serde_json::json!({})).await;
    assert_eq!(response.error.unwrap().code, -32013);
    let denied_fixture =
        fixture_with_policy("live-admission", RateLimitConfig::default(), true, policy).await;
    let mut denied = connect(&denied_fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    assert!(matches!(
        denied.next().await,
        Some(Ok(Message::Close(_))) | None
    ));
    assert_eq!(denied_fixture.pool.len().await, 0);
    denied_fixture.controller.stop().await.unwrap();
    fixture.controller.stop().await.unwrap();
}

#[tokio::test]
async fn hanging_policy_is_bounded_and_cannot_hold_an_idle_socket_forever() {
    let policy = Arc::new(RevocablePolicy::default());
    let fixture = fixture_with_policy(
        "live-timeout",
        RateLimitConfig::default(),
        true,
        policy.clone(),
    )
    .await;
    let mut owner = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    assert!(
        call(&mut owner, "hosted/whoami", serde_json::json!({}))
            .await
            .error
            .is_none()
    );
    policy.hanging.store(true, Ordering::Release);
    let reason = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            match owner.next().await {
                Some(Ok(Message::Text(text))) => {
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if value.get("error").is_some() {
                        break value["error"]["message"].as_str().unwrap().to_string();
                    }
                }
                other => panic!("closed before terminal error: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
    assert!(reason.contains("connection_authorization_timeout"));
    fixture.controller.stop().await.unwrap();
}
