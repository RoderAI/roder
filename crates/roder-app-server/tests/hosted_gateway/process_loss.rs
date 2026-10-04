//! Unlike orderly shutdown, a killed owner cannot terminalize its pending calls.
//! The successor must expose recovery state without replaying browser effects.
use super::*;
use roder_api::extension::RoderExtension;
use roder_app_server::hosted::HostedRuntimeLeaseBackend;
use roder_core::RuntimeExecutionLease;
use roder_ext_mysql_session::ownership::{RuntimeOwnerClaim, RuntimeOwnerLease};
use roder_ext_mysql_session::{MysqlSessionConfig, MysqlSessionExtension, MysqlSessionStore};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Backend {
    store: MysqlSessionStore,
    lease: RuntimeOwnerLease,
}
#[async_trait::async_trait]
impl HostedRuntimeLeaseBackend for Backend {
    async fn release(&self) -> anyhow::Result<bool> {
        self.store.release_runtime_owner(&self.lease).await
    }
    async fn renew(&self, ttl: Duration) -> anyhow::Result<bool> {
        Ok(self
            .store
            .renew_runtime_owner(&self.lease, ttl)
            .await?
            .is_some())
    }
}

// Invoked only by the parent test. All connection secrets remain in inherited env.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "subprocess fixture for the isolated MySQL process-loss test"]
async fn mysql_gateway_child() {
    let Ok(ready_file) = std::env::var("RODER_CRASH_READY_FILE") else {
        return;
    };
    let namespace = std::env::var("RODER_CRASH_NAMESPACE").unwrap();
    let endpoint = Arc::new(std::sync::Mutex::new("127.0.0.1:1".parse().unwrap()));
    let factory_endpoint = endpoint.clone();
    let pool = Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile {
            data_root: temp_dir("process-loss"),
            allow_local_workspaces: true,
            ..Default::default()
        },
        Arc::new(move |tenant, _| {
            let namespace = namespace.clone();
            let endpoint = *factory_endpoint.lock().unwrap();
            Box::pin(async move {
                let config = MysqlSessionConfig::new(
                    std::env::var("RODER_MYSQL_TEST_URL")?,
                    format!("{namespace}-{tenant}"),
                )?;
                let store = MysqlSessionStore::connect(&config).await?;
                let ttl = Duration::from_secs(3);
                let requested_at = Instant::now();
                let lease = match store
                    .claim_runtime_owner(uuid::Uuid::new_v4(), endpoint, ttl)
                    .await?
                {
                    RuntimeOwnerClaim::Acquired(lease) => lease,
                    RuntimeOwnerClaim::Occupied(owner) => {
                        return Err(roder_app_server::hosted::HostedRuntimeRedirect {
                            tenant_id: tenant,
                            endpoint: owner.endpoint,
                        }
                        .into());
                    }
                };
                let authority = Arc::new(RuntimeExecutionLease::new(requested_at + ttl));
                let mut registry = ExtensionRegistryBuilder::new();
                registry.inference_engine(Arc::new(FakeInferenceEngine));
                MysqlSessionExtension::from_store(store.with_runtime_owner(&lease)?)
                    .install(&mut registry)?;
                let server = Arc::new(AppServer::new(Arc::new(
                    Runtime::new(registry.build()?, RuntimeConfig::default())?
                        .with_execution_lease(authority.clone()),
                )));
                server.supervise_runtime_lease(
                    authority,
                    Arc::new(Backend { store, lease }),
                    ttl,
                )?;
                Ok(server)
            })
        }),
    ));
    let fixture = fixture_with_pool(
        pool,
        RateLimitConfig::default(),
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await;
    *endpoint.lock().unwrap() = fixture.controller.listen_addr;
    std::fs::write(ready_file, fixture.url).unwrap();
    std::future::pending::<()>().await;
}

async fn spawn_gateway(namespace: &str) -> (KillOnDrop, String) {
    let dir = temp_dir("crash-ready");
    let ready = dir.join("ready");
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "process_loss::mysql_gateway_child",
            "--ignored",
            "--nocapture",
        ])
        .env("RODER_CRASH_NAMESPACE", namespace)
        .env("RODER_CRASH_READY_FILE", &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut child = KillOnDrop(child);
    let url = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(url) = std::fs::read_to_string(&ready) {
                break url;
            }
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "gateway child exited before ready"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("gateway startup deadline");
    std::fs::remove_dir_all(dir).unwrap();
    (child, url)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires isolated RODER_MYSQL_TEST_URL; kills only its own child gateway"]
async fn killed_gateway_recovers_thread_without_replaying_pending_browser_call() {
    let namespace = format!("crash-{}", uuid::Uuid::new_v4());
    let store = MysqlSessionStore::connect(
        &MysqlSessionConfig::new(
            std::env::var("RODER_MYSQL_TEST_URL").unwrap(),
            format!("{namespace}-tenant-a"),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let (mut owner, owner_url) = spawn_gateway(&namespace).await;
    let mut socket = connect(&owner_url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let directory = temp_dir("crash-workspace");
    let workspace = call(
        &mut socket,
        "workspace/create",
        serde_json::json!({
            "roots":[{"path":directory}], "defaultRootPath":directory
        }),
    )
    .await
    .result
    .unwrap()["workspace"]
        .clone();
    let thread = call(
        &mut socket,
        "thread/start",
        serde_json::json!({
            "workspaceId":workspace["id"], "model":"mock", "externalTools":[{
                "name":"acme_lookup", "description":"lookup", "parameters":{"type":"object"}
            }]
        }),
    )
    .await
    .result
    .unwrap()["thread"]["id"]
        .clone();
    let lease = call(
        &mut socket,
        "tools/bind_executor",
        serde_json::json!({"threadId":thread}),
    )
    .await
    .result
    .unwrap()["executor"]
        .clone();
    call(
        &mut socket,
        "turn/start",
        serde_json::json!({"threadId":thread,"prompt":"FAKE_EXTERNAL_TOOL lookup"}),
    )
    .await
    .result
    .unwrap();
    let pending =
        super::hosted_executor::notification(&mut socket, "thread/toolExecutionRequested").await;
    let old_owner = store.runtime_owner().await.unwrap().unwrap();
    let (_standby, url) = spawn_gateway(&namespace).await;
    let mut routed = connect(&url, "rk_test_tenant_a_writer").await.unwrap();
    let before = call(
        &mut routed,
        "thread/read",
        serde_json::json!({"threadId":thread}),
    )
    .await;
    assert_eq!(before.result.unwrap()["thread"]["id"], thread);
    let routed_owner = store.runtime_owner().await.unwrap().unwrap();
    assert_eq!(routed_owner.owner_id, old_owner.owner_id);
    assert_eq!(routed_owner.generation, old_owner.generation);
    assert_eq!(routed_owner.endpoint, old_owner.endpoint);
    // kill() uses SIGKILL on Unix, bypassing gateway shutdown and lease release.
    owner.0.kill().unwrap();
    assert!(!owner.0.wait().unwrap().success());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                _ => {}
            }
        }
    })
    .await
    .expect("dead gateway socket must close");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match routed.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                _ => {}
            }
        }
    })
    .await
    .expect("relay must close after abrupt owner loss");
    assert_eq!(
        store.runtime_owner().await.unwrap().unwrap().generation,
        old_owner.generation
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while store.runtime_owner().await.unwrap().is_some() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("killed owner lease expiry");
    let mut recovered = connect(&url, "rk_test_tenant_a_writer").await.unwrap();
    let read = call(
        &mut recovered,
        "thread/read",
        serde_json::json!({"threadId":thread,"includeTurns":true}),
    )
    .await;
    assert!(read.error.is_none(), "{:?}", read.error);
    let result = read.result.unwrap();
    assert_eq!(result["thread"]["id"], thread);
    let lifecycle = result["lifecycle"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["turnId"] == pending["turnId"])
        .expect("crashed turn recovery record");
    assert_eq!(lifecycle["state"], "recovery_needed");
    assert_eq!(lifecycle["reason"], "runtime_restart");
    assert_eq!(lifecycle["cleanup"], "unknown");
    let new_owner = store.runtime_owner().await.unwrap().unwrap();
    assert!(new_owner.generation > old_owner.generation);
    assert_ne!(new_owner.owner_id, old_owner.owner_id);
    let replay = call(&mut recovered, "tools/resolve", serde_json::json!({
        "executor":lease, "turnId":pending["turnId"], "requestId":pending["requestId"], "output":"old result", "isError":false
    })).await;
    assert!(
        replay.error.is_some(),
        "old process lease cannot resolve after crash"
    );
    let fresh = call(
        &mut recovered,
        "tools/bind_executor",
        serde_json::json!({"threadId":thread}),
    )
    .await;
    assert!(fresh.error.is_none(), "{:?}", fresh.error);
    let fresh_lease = fresh.result.unwrap()["executor"].clone();
    assert_ne!(fresh_lease["leaseId"], lease["leaseId"]);
    let receipt = call(
        &mut recovered,
        "tools/execution_read",
        serde_json::json!({
            "executor":fresh_lease, "requestId":pending["requestId"]
        }),
    )
    .await
    .result
    .unwrap()["execution"]
        .clone();
    assert_eq!(
        receipt["state"], "uncertain",
        "a killed process cannot prove whether the browser applied its pending action"
    );
    assert_eq!(receipt["turnId"], pending["turnId"]);
    let late = call(&mut recovered, "tools/resolve", serde_json::json!({
        "executor":fresh_lease, "turnId":pending["turnId"], "requestId":pending["requestId"], "output":"late result", "isError":false
    })).await;
    assert!(
        late.error.is_some(),
        "a recovered receipt never grants execution ownership"
    );
    let quiet = tokio::time::timeout(Duration::from_millis(250), async {
        while let Some(Ok(Message::Text(text))) = recovered.next().await {
            let event: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_ne!(
                event["method"], "thread/toolExecutionRequested",
                "history must never replay effects"
            );
        }
    })
    .await;
    assert!(quiet.is_err(), "successor connection should remain usable");
    assert!(
        call(&mut recovered, "hosted/whoami", serde_json::json!({}))
            .await
            .error
            .is_none()
    );
    println!(
        "crash recovery: pending request {}, ownership generation {} -> {}, old resolve rejected, no historical tool replay",
        pending["requestId"], old_owner.generation, new_owner.generation
    );
}
