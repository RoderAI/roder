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
    pub async fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        for revoked in self
            .server
            .external_tool_executors
            .disconnect(&self.id)
            .await
        {
            revoke(&self.server, revoked, "disconnected").await;
        }
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
