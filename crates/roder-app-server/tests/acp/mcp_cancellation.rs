use super::*;

#[derive(Default)]
struct PendingPermission(tokio::sync::Notify);

#[async_trait]
impl AcpClientPeer for PendingPermission {
    async fn send_notification(&self, _: JsonRpcNotification) -> anyhow::Result<()> {
        Ok(())
    }

    async fn request_permission(
        &self,
        _: acp::RequestPermissionRequest,
    ) -> anyhow::Result<acp::RequestPermissionResponse> {
        self.0.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn cancellation_drops_unanswered_mcp_permission_without_executing() {
    let root = tempfile::tempdir().unwrap();
    let fixture = mcp_fixture::Server::start().await;
    let adapter = Arc::new(adapter(root.path(), Default::default()));
    let peer = Arc::new(PendingPermission::default());
    let session = new_session(
        &adapter,
        &*peer,
        json!({"cwd":root.path(),"mcpServers":fixture.config()}),
    )
    .await;
    let id = session["sessionId"].as_str().unwrap().to_string();
    let running = {
        let adapter = adapter.clone();
        let peer = peer.clone();
        let id = id.clone();
        tokio::spawn(async move {
            rpc(
                &adapter,
                &*peer,
                "session/prompt",
                json!({"sessionId":id,"prompt":[{"type":"text","text":"MCP"}]}),
            )
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), peer.0.notified())
        .await
        .unwrap();
    let cancelled = adapter
        .handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: None,
                method: "session/cancel".into(),
                params: Some(json!({"sessionId":id})),
            },
            &*peer,
        )
        .await
        .unwrap();
    assert!(cancelled.is_none());
    let response = tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.result.unwrap()["stopReason"], "cancelled");
    assert!(fixture.calls.lock().await.is_empty());
    adapter.shutdown().await.unwrap();
}
