//! Remote agent-node awareness for the TUI (roadmap phase 67, Stage 4).
//!
//! When the TUI drives a remote node (via `roder agent-node connect`), the
//! node — not this terminal — is the source of truth for sessions, tools,
//! and files. `announce_remote_node` queries `node/status` once after
//! startup and renders an authority banner so that stays unmistakable.

use roder_app_server::AppClient;
use roder_protocol::JsonRpcRequest;
use roder_protocol::agent_node::NodeStatusResult;

use super::TuiApp;

impl<C> TuiApp<C>
where
    C: AppClient,
{
    /**
     * Queries `node/status` and, when served by a remote agent node,
     * pushes a banner naming the node, fingerprint, and auth mode. Safe to
     * call against local app-servers (renders nothing). Returns the node
     * label when remote.
     */
    pub async fn announce_remote_node(&mut self) -> Option<String> {
        let response = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("node-status")),
                method: "node/status".to_string(),
                params: Some(serde_json::json!({})),
            })
            .await;
        let status: NodeStatusResult = serde_json::from_value(response.result?).ok()?;
        if !status.served {
            return None;
        }
        let node = status.node?;
        let label = format!(
            "{} ({})",
            node.name,
            &node.node_id[..node.node_id.len().min(12)]
        );
        let mut lines = vec![format!(
            "Remote agent node: {label} — turns, tools, and files run on the node, not this \
             terminal."
        )];
        lines.push(format!(
            "auth {}  fingerprint {}  protocol {}",
            node.auth_mode.as_deref().unwrap_or("?"),
            short_fingerprint(&node.fingerprint),
            node.protocol_version
        ));
        if let Some(workspace) = &node.workspace {
            lines.push(format!("node workspace {workspace}"));
        }
        self.timeline.push_system(lines.join("\n"));
        self.push_event(format!("remote node: {label}"));
        Some(label)
    }
}

fn short_fingerprint(fingerprint: &str) -> String {
    if fingerprint.len() <= 16 {
        fingerprint.to_string()
    } else {
        format!(
            "{}…{}",
            &fingerprint[..8],
            &fingerprint[fingerprint.len() - 8..]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_render_compactly() {
        assert_eq!(short_fingerprint("abc123"), "abc123");
        let long = "0123456789abcdef0123456789abcdef";
        let short = short_fingerprint(long);
        assert!(short.starts_with("01234567") && short.ends_with("89abcdef"));
        assert!(short.len() < long.len());
    }
}
