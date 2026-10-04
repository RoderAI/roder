//! Hosted executor dispatch and disconnect cleanup, after gateway authorization.

use super::external_tools::RevokedExecutor;
use crate::AppServer;
use roder_protocol::{
    JsonRpcError, JsonRpcRequest, JsonRpcResponse, ToolsBindExecutorParams,
    ToolsExecutionReadParams, ToolsResolveParams, ToolsUnbindExecutorParams,
};
use std::sync::Arc;

pub(crate) struct ExecutorConnection {
    server: Arc<AppServer>,
    pub id: String,
    closed: bool,
}

impl ExecutorConnection {
    pub fn new(server: Arc<AppServer>) -> Self {
        server.external_tool_executors.monitor(&server);
        Self {
            server,
            id: uuid::Uuid::new_v4().to_string(),
            closed: false,
        }
    }
}

impl ExecutorConnection {
    pub async fn revoke(&mut self, reason: &str) {
        self.cleanup("cancelled", reason).await;
    }

    pub async fn close(&mut self) {
        self.cleanup("disconnected", "disconnected").await;
    }

    async fn cleanup(&mut self, state: &'static str, reason: &str) {
        if self.closed {
            return;
        }
        let server = self.server.clone();
        let connection = self.id.clone();
        let reason = reason.to_string();
        // Spawn the complete transition before awaiting it. If the gateway
        // task is aborted, dropping this JoinHandle does not abort cleanup.
        let cleanup = tokio::spawn(async move {
            for revoked in server
                .external_tool_executors
                .revoke_connection(&connection, state)
                .await
            {
                revoke(&server, revoked, &reason).await;
            }
        });
        self.closed = true;
        let _ = cleanup.await;
    }
}

impl Drop for ExecutorConnection {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let server = self.server.clone();
        let connection = self.id.clone();
        // Also runs when the gateway aborts this connection task on shutdown.
        tokio::spawn(async move {
            for revoked in server.external_tool_executors.disconnect(&connection).await {
                revoke(&server, revoked, "disconnected").await;
            }
        });
    }
}

pub(crate) async fn reject_execution(server: &AppServer, request: &str, reason: &str) {
    let _ = server.runtime.resolve_external_tool_call(request, roder_core::ExternalToolResolution {
        output: serde_json::json!({"error":reason,"recovery":"Read current page state before proposing a new action. Do not replay this request."}).to_string(),
        is_error: true,
    }).await;
}

pub(super) async fn revoke(server: &AppServer, revoked: RevokedExecutor, reason: &str) {
    server.publish_notification(roder_protocol::JsonRpcNotification {
        jsonrpc: "2.0".into(),
        method: "tools/executorRevoked".into(),
        params: serde_json::json!({"executor":revoked.lease,"reason":reason}),
    });
    for request in revoked.requests {
        reject_execution(server, &request, reason).await;
    }
}

fn failure(request: &JsonRpcRequest, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id: request.id.clone(),
        result: None,
        error: Some(JsonRpcError {
            code: -32012,
            message: message.into(),
            data: None,
        }),
    }
}

async fn thread_has_tools(server: &AppServer, thread: &str) -> Result<bool, String> {
    let response = server
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: "thread/read".into(),
            params: Some(serde_json::json!({"threadId":thread,"includeTurns":false})),
        })
        .await;
    let value = response
        .result
        .as_ref()
        .and_then(|v| v.get("thread"))
        .filter(|v| !v.is_null())
        .ok_or("thread_not_found")?;
    Ok(value
        .get("externalTools")
        .and_then(|v| v.as_array())
        .is_some_and(|v| !v.is_empty()))
}

pub(crate) async fn dispatch(
    server: &AppServer,
    connection: &str,
    request: &JsonRpcRequest,
) -> Option<JsonRpcResponse> {
    let params = request.params.clone().unwrap_or_default();
    let outcome: Result<serde_json::Value, String> = match request.method.as_str() {
        "tools/bind_executor" => match serde_json::from_value::<ToolsBindExecutorParams>(params) {
            Err(error) => Err(error.to_string()),
            Ok(params) => match thread_has_tools(server, &params.thread_id).await {
                Ok(true) => match server
                    .external_tool_executors
                    .bind(connection, &params.thread_id, params.takeover)
                    .await
                {
                    Ok((executor, revoked)) => {
                        if let Some(revoked) = revoked {
                            revoke(server, revoked, "taken_over").await;
                        }
                        Ok(serde_json::json!({"executor":executor}))
                    }
                    Err(error) => Err(error.into()),
                },
                Ok(false) => Err("thread_has_no_external_tools".into()),
                Err(error) => Err(error),
            },
        },
        "tools/unbind_executor" => {
            match serde_json::from_value::<ToolsUnbindExecutorParams>(params) {
                Err(error) => Err(error.to_string()),
                Ok(params) => match server
                    .external_tool_executors
                    .unbind(connection, &params.executor)
                    .await
                {
                    Ok(revoked) => {
                        if let Some(revoked) = revoked {
                            revoke(server, revoked, "unbound").await;
                        }
                        Ok(serde_json::json!({"unbound":true}))
                    }
                    Err(error) => Err(error.into()),
                },
            }
        }
        "tools/execution_read" => {
            match serde_json::from_value::<ToolsExecutionReadParams>(params) {
                Err(error) => Err(error.to_string()),
                Ok(params) => server
                    .external_tool_executors
                    .read_with_history(
                        connection,
                        &params.executor,
                        &params.request_id,
                        &server.runtime,
                    )
                    .await
                    .map(|execution| serde_json::json!({"execution":execution})),
            }
        }
        "tools/resolve" => match serde_json::from_value::<ToolsResolveParams>(params) {
            Err(error) => Err(error.to_string()),
            Ok(params) => match server
                .external_tool_executors
                .resolve(connection, params, &server.runtime)
                .await
            {
                Ok(resolved) => Ok(serde_json::json!({"resolved":resolved})),
                Err(error) => Err(error.into()),
            },
        },
        "turn/start" => {
            let thread = params
                .get("threadId")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            match thread_has_tools(server, thread).await {
                Ok(true)
                    if !server
                        .external_tool_executors
                        .owns_thread(connection, thread)
                        .await =>
                {
                    Err("executor_required".into())
                }
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(match outcome {
        Ok(result) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: request.id.clone(),
            result: Some(result),
            error: None,
        },
        Err(error) => failure(request, error),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_core::{Runtime, RuntimeConfig, fake_provider::FakeInferenceEngine};
    use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
    use serde_json::{Value, json};

    async fn rpc(server: &AppServer, method: &str, params: Value) -> Value {
        let response = server
            .handle_request(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(method)),
                method: method.into(),
                params: Some(params),
            })
            .await;
        assert!(response.error.is_none(), "{:?}", response.error);
        response.result.unwrap()
    }

    #[tokio::test]
    async fn aborting_revocation_after_its_first_poll_still_terminalizes_the_pending_turn() {
        let directory = tempfile::tempdir().unwrap();
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: directory.path().join("threads"),
        }));
        let server = Arc::new(AppServer::new(Arc::new(
            Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap(),
        )));
        let workspace = rpc(
            &server,
            "workspace/create",
            json!({"roots":[{"path":directory.path()}],"defaultRootPath":directory.path()}),
        )
        .await["workspace"]
            .clone();
        let thread = rpc(&server, "thread/start", json!({"workspaceId":workspace["id"],"model":"mock",
            "externalTools":[{"name":"acme_lookup","description":"lookup","parameters":{"type":"object"}}]
        })).await["thread"]["id"].as_str().unwrap().to_string();
        let mut connection = ExecutorConnection::new(server.clone());
        server
            .external_tool_executors
            .bind(&connection.id, &thread, false)
            .await
            .unwrap();
        let mut notifications = server.subscribe_notifications();
        rpc(
            &server,
            "turn/start",
            json!({"threadId":thread,"prompt":"FAKE_EXTERNAL_TOOL lookup"}),
        )
        .await;
        let request = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let notification = notifications.recv().await.unwrap();
                if notification.method == "thread/toolExecutionRequested" {
                    let request = notification.params["requestId"]
                        .as_str()
                        .unwrap()
                        .to_string();
                    server
                        .external_tool_executors
                        .observe(&connection.id, notification)
                        .await;
                    break request;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(server.runtime.active_turn_count().await, 1);
        // On this current-thread runtime, the detached task cannot run until
        // we yield. Cancel the caller after spawning it but before cleanup.
        let mut cleanup = Box::pin(connection.revoke("authorization_revoked"));
        assert!(futures::poll!(cleanup.as_mut()).is_pending());
        drop(cleanup);
        drop(connection);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while server.runtime.active_turn_count().await != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let (lease, _) = server
            .external_tool_executors
            .bind("fresh", &thread, false)
            .await
            .unwrap();
        let state = server
            .external_tool_executors
            .read("fresh", &lease, &request)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.state, "cancelled");
    }
}
