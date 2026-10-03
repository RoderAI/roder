use super::*;
use std::sync::Arc;

impl ExecutorBindings {
    pub fn monitor(self: &Arc<Self>, server: &Arc<crate::AppServer>) {
        self.monitor.get_or_init(|| {
            let bindings = Arc::downgrade(self);
            let server = Arc::downgrade(server);
            let mut notifications = server.upgrade().unwrap().subscribe_notifications();
            tokio::spawn(async move {
                loop {
                    let notification = notifications.recv().await;
                    let (Some(bindings), Some(server)) = (bindings.upgrade(), server.upgrade())
                    else {
                        break;
                    };
                    match notification {
                        Ok(notification) => {
                            if let Delivery::Reject(request) =
                                bindings.observe("", notification).await
                            {
                                super::super::executor_gateway::reject_execution(
                                    &server,
                                    &request,
                                    "executor_unavailable",
                                )
                                .await;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            bindings.reconcile_lag(&server).await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
            .abort_handle()
        });
    }

    /// Losing execution notifications makes effects uncertain. Revoke ownership,
    /// cancel every runtime-pending call, and require fresh context and binding.
    async fn reconcile_lag(&self, server: &crate::AppServer) {
        let pending = server.runtime.pending_external_executions().await;
        let revoked = {
            let mut bindings = self.inner.lock().await;
            let threads = bindings.leases.keys().cloned().collect::<Vec<_>>();
            let revoked = threads
                .iter()
                .filter_map(|thread| bindings.revoke(thread, "uncertain"))
                .collect::<Vec<_>>();
            for call in bindings
                .calls
                .values_mut()
                .filter(|call| call.state.state == "pending")
            {
                call.state.state = "uncertain".into();
                call.state.is_error = true;
                call.updated = Instant::now();
            }
            for execution in &pending {
                bindings
                    .calls
                    .entry(execution.request_id.clone())
                    .or_insert_with(|| Call {
                        executor: None,
                        updated: Instant::now(),
                        state: ToolExecutionState {
                            thread_id: execution.thread_id.clone(),
                            turn_id: execution.turn_id.clone(),
                            request_id: execution.request_id.clone(),
                            tool_name: execution.tool_name.clone(),
                            state: "uncertain".into(),
                            is_error: true,
                        },
                    });
            }
            bindings.prune();
            revoked
        };
        for executor in revoked {
            server.publish_notification(JsonRpcNotification {
                jsonrpc: "2.0".into(),
                method: "tools/executorRevoked".into(),
                params: serde_json::json!({"executor":executor.lease,"reason":"notification_lag"}),
            });
        }
        for execution in pending {
            super::super::executor_gateway::reject_execution(
                server,
                &execution.request_id,
                "notification_lag",
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_protocol::{JsonRpcRequest, ToolsResolveParams};

    async fn request(
        server: &crate::AppServer,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        let response = server
            .handle_request(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(serde_json::json!(1)),
                method: method.into(),
                params: Some(params),
            })
            .await;
        assert!(response.error.is_none(), "{:?}", response.error);
        response.result.unwrap()
    }

    #[tokio::test]
    async fn lag_reconciles_real_pending_calls_and_never_replays_missing_requests() {
        for missed in [false, true] {
            let directory =
                std::env::temp_dir().join(format!("roder-executor-lag-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&directory).unwrap();
            let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
            builder.inference_engine(Arc::new(roder_core::fake_provider::FakeInferenceEngine));
            builder.thread_store_factory(Arc::new(
                roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory {
                    base_path: directory.join("threads"),
                },
            ));
            let runtime = Arc::new(
                roder_core::Runtime::new(
                    builder.build().unwrap(),
                    roder_core::RuntimeConfig::default(),
                )
                .unwrap(),
            );
            let server = Arc::new(crate::AppServer::new(runtime.clone()));
            let bindings = server.external_tool_executors.clone();
            bindings.monitor(&server);

            let workspace = request(
                &server,
                "workspace/create",
                serde_json::json!({"roots":[{"path":directory}]}),
            )
            .await;
            let thread = request(&server,"thread/start",serde_json::json!({
                "workspaceId":workspace["workspace"]["id"],"rootId":workspace["workspace"]["defaultRootId"],"model":"mock",
                "externalTools":[{"name":"acme_lookup","description":"lookup","parameters":{"type":"object"}}]
            })).await["thread"]["id"].as_str().unwrap().to_string();
            let (old, _) = bindings.bind("owner", &thread, false).await.unwrap();
            let mut notifications = server.subscribe_notifications();
            request(
                &server,
                "turn/start",
                serde_json::json!({"threadId":thread,"prompt":"FAKE_EXTERNAL_TOOL lookup"}),
            )
            .await;
            let execution = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let notification = notifications.recv().await.unwrap();
                    if notification.method == "thread/toolExecutionRequested" {
                        break notification.params;
                    }
                }
            })
            .await
            .unwrap();
            let id = execution["requestId"].as_str().unwrap();
            if missed {
                bindings.inner.lock().await.calls.remove(id);
            }
            bindings.reconcile_lag(&server).await;
            assert!(runtime.pending_external_executions().await.is_empty());
            let (new, _) = bindings.bind("new", &thread, false).await.unwrap();
            assert_eq!(
                bindings.read("new", &new, id).await.unwrap().unwrap().state,
                "uncertain"
            );
            assert!(
                bindings
                    .resolve(
                        "owner",
                        ToolsResolveParams {
                            request_id: id.into(),
                            turn_id: Some(execution["turnId"].as_str().unwrap().into()),
                            executor: Some(old),
                            output: "late".into(),
                            is_error: false
                        },
                        &runtime
                    )
                    .await
                    .is_err()
            );
            std::fs::remove_dir_all(directory).unwrap();
        }
    }
}
