use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::HonchoMemoryConfig;

/// Thin wrapper over Honcho's v3 REST API covering only the endpoints the
/// memory store uses. Workspace/peer/session creation endpoints are
/// get-or-create, so ensure calls are idempotent.
pub(crate) struct HonchoClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct HonchoMessage {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub peer_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PageResponse<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct SessionResponse {
    id: String,
}

impl HonchoClient {
    pub fn new(config: &HonchoMemoryConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
        }
    }

    pub async fn ensure_workspace(&self, workspace: &str) -> anyhow::Result<()> {
        self.post_json(
            "/v3/workspaces",
            &json!({ "id": workspace }),
            "ensure workspace",
        )
        .await?;
        Ok(())
    }

    pub async fn ensure_peer(&self, workspace: &str, peer: &str) -> anyhow::Result<()> {
        self.post_json(
            &format!("/v3/workspaces/{workspace}/peers"),
            &json!({ "id": peer }),
            "ensure peer",
        )
        .await?;
        Ok(())
    }

    pub async fn ensure_session(
        &self,
        workspace: &str,
        session: &str,
        peer: &str,
        metadata: Value,
    ) -> anyhow::Result<()> {
        self.post_json(
            &format!("/v3/workspaces/{workspace}/sessions"),
            &json!({
                "id": session,
                "metadata": metadata,
                "peers": { peer: {} },
            }),
            "ensure session",
        )
        .await?;
        Ok(())
    }

    pub async fn add_message(
        &self,
        workspace: &str,
        session: &str,
        peer: &str,
        content: &str,
        metadata: Value,
        created_at: &str,
    ) -> anyhow::Result<HonchoMessage> {
        let value = self
            .post_json(
                &format!("/v3/workspaces/{workspace}/sessions/{session}/messages"),
                &json!({
                    "messages": [{
                        "peer_id": peer,
                        "content": content,
                        "metadata": metadata,
                        "created_at": created_at,
                    }],
                }),
                "add message",
            )
            .await?;
        let mut messages: Vec<HonchoMessage> =
            serde_json::from_value(value).context("decode add message response")?;
        let mut message = match messages.pop() {
            Some(message) => message,
            None => bail!("add message returned no messages"),
        };
        if message.session_id.is_empty() {
            message.session_id = session.to_string();
        }
        Ok(message)
    }

    pub async fn get_message(
        &self,
        workspace: &str,
        session: &str,
        message: &str,
    ) -> anyhow::Result<Option<HonchoMessage>> {
        let url = format!(
            "{}/v3/workspaces/{workspace}/sessions/{session}/messages/{message}",
            self.base_url
        );
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .context("get message")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let value = decode_response(response, "get message").await?;
        let mut message: HonchoMessage =
            serde_json::from_value(value).context("decode get message response")?;
        if message.session_id.is_empty() {
            message.session_id = session.to_string();
        }
        Ok(Some(message))
    }

    pub async fn update_message_metadata(
        &self,
        workspace: &str,
        session: &str,
        message: &str,
        metadata: Value,
    ) -> anyhow::Result<()> {
        let url = format!(
            "{}/v3/workspaces/{workspace}/sessions/{session}/messages/{message}",
            self.base_url
        );
        let response = self
            .http
            .put(url)
            .bearer_auth(&self.api_key)
            .json(&json!({ "metadata": metadata }))
            .send()
            .await
            .context("update message metadata")?;
        decode_response(response, "update message metadata").await?;
        Ok(())
    }

    pub async fn search_workspace(
        &self,
        workspace: &str,
        query: &str,
        filters: Value,
        limit: usize,
    ) -> anyhow::Result<Vec<HonchoMessage>> {
        let value = self
            .post_json(
                &format!("/v3/workspaces/{workspace}/search"),
                &json!({ "query": query, "filters": filters, "limit": limit }),
                "search workspace",
            )
            .await?;
        serde_json::from_value(value).context("decode search response")
    }

    /// Returns an empty list when the session does not exist yet.
    pub async fn list_session_messages(
        &self,
        workspace: &str,
        session: &str,
        filters: Value,
        size: usize,
    ) -> anyhow::Result<Vec<HonchoMessage>> {
        let url = format!(
            "{}/v3/workspaces/{workspace}/sessions/{session}/messages/list",
            self.base_url
        );
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&json!({ "filters": filters, "size": size }))
            .send()
            .await
            .context("list session messages")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        let value = decode_response(response, "list session messages").await?;
        let page: PageResponse<HonchoMessage> =
            serde_json::from_value(value).context("decode list messages response")?;
        let mut messages = page.items;
        for message in &mut messages {
            if message.session_id.is_empty() {
                message.session_id = session.to_string();
            }
        }
        Ok(messages)
    }

    pub async fn list_sessions(
        &self,
        workspace: &str,
        filters: Value,
        size: usize,
    ) -> anyhow::Result<Vec<String>> {
        let value = self
            .post_json(
                &format!("/v3/workspaces/{workspace}/sessions/list"),
                &json!({ "filters": filters, "size": size }),
                "list sessions",
            )
            .await?;
        let page: PageResponse<SessionResponse> =
            serde_json::from_value(value).context("decode list sessions response")?;
        Ok(page.items.into_iter().map(|session| session.id).collect())
    }

    async fn post_json(&self, path: &str, body: &Value, operation: &str) -> anyhow::Result<Value> {
        let response = self
            .http
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(&self.api_key)
            .json(body)
            .send()
            .await
            .context(operation.to_string())?;
        decode_response(response, operation).await
    }
}

async fn decode_response(response: reqwest::Response, operation: &str) -> anyhow::Result<Value> {
    let status = response.status();
    let body = response.bytes().await?;
    if !status.is_success() {
        bail!("{operation} failed with {status}: {}", redact_body(&body));
    }
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&body).with_context(|| format!("decode {operation} response"))
}

/// Error bodies may echo credentials back; strip secret-looking markers
/// before they can reach logs.
fn redact_body(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let mut out = text.to_string();
    for marker in ["token", "api_key", "secret", "authorization", "bearer"] {
        out = out.replace(marker, "<redacted>");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secret_error_bodies() {
        let body = br#"{"error":"authorization bearer token api_key secret leaked"}"#;
        let redacted = redact_body(body);

        assert!(!redacted.contains("authorization"));
        assert!(!redacted.contains("bearer"));
        assert!(!redacted.contains("token"));
        assert!(!redacted.contains("api_key"));
        assert!(!redacted.contains("secret"));
    }
}
