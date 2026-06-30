use std::time::Duration;

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::BlaxelConfig;

/// Thin HTTP client for the Blaxel control plane (`/sandboxes`) and the
/// per-sandbox REST API (process/filesystem/preview) reached through the
/// sandbox's auto-generated endpoint URL.
#[derive(Clone)]
pub struct BlaxelClient {
    http: reqwest::Client,
    token: String,
    workspace: Option<String>,
    base_url: String,
}

/// A Blaxel sandbox as returned by the control plane.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Sandbox {
    #[serde(default)]
    pub metadata: SandboxMetadata,
    /// `RUNNING` or `STANDBY` (read-only, system-managed).
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SandboxMetadata {
    #[serde(default)]
    pub name: String,
    /// Auto-generated endpoint URL used to reach the per-sandbox API.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "externalId")]
    pub external_id: Option<String>,
}

impl Sandbox {
    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    pub fn endpoint_url(&self) -> Option<&str> {
        self.metadata.url.as_deref()
    }

    pub fn is_terminated(&self) -> bool {
        matches!(
            self.status.as_deref(),
            Some("TERMINATED") | Some("DELETING") | Some("FAILED")
        )
    }
}

/// Result of executing a command in the sandbox.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProcessResponse {
    #[serde(default, rename = "exitCode")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FileWithContent {
    #[serde(default)]
    content: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct Preview {
    #[serde(default)]
    spec: PreviewSpec,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct PreviewSpec {
    #[serde(default)]
    url: Option<String>,
}

impl BlaxelClient {
    pub fn new(http: reqwest::Client, config: &BlaxelConfig) -> Self {
        Self {
            http,
            token: config.token.as_str().to_string(),
            workspace: config.workspace.clone(),
            base_url: config.base_url.clone(),
        }
    }

    fn control(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let mut builder = self.http.request(method, url).bearer_auth(&self.token);
        if let Some(workspace) = &self.workspace {
            builder = builder.header("X-Blaxel-Workspace", workspace);
        }
        builder
    }

    fn sandbox(
        &self,
        endpoint: &str,
        method: reqwest::Method,
        path: &str,
    ) -> reqwest::RequestBuilder {
        let url = format!(
            "{}/{}",
            endpoint.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        self.http.request(method, url).bearer_auth(&self.token)
    }

    // ---- control plane -------------------------------------------------

    /// Create a sandbox, returning the existing one when it already exists
    /// (`createIfNotExist=true`).
    pub async fn create_sandbox(
        &self,
        name: &str,
        config: &BlaxelConfig,
    ) -> anyhow::Result<Sandbox> {
        let mut metadata = json!({ "name": name });
        if let Some(external_id) = &config.external_id {
            metadata["externalId"] = json!(external_id);
        }
        let mut runtime = json!({
            "image": config.image,
            "memory": config.memory_mb,
        });
        if let Some(ttl) = &config.ttl {
            runtime["ttl"] = json!(ttl);
        }
        let mut spec = json!({ "runtime": runtime });
        if let Some(region) = &config.region {
            spec["region"] = json!(region);
        }
        let body = json!({ "metadata": metadata, "spec": spec });
        let response = self
            .control(reqwest::Method::POST, "sandboxes?createIfNotExist=true")
            .json(&body)
            .send()
            .await
            .context("create blaxel sandbox")?;
        decode(response, "create sandbox").await
    }

    pub async fn get_sandbox(&self, name: &str) -> anyhow::Result<Sandbox> {
        let response = self
            .control(
                reqwest::Method::GET,
                &format!("sandboxes/{}", urlencoding::encode(name)),
            )
            .send()
            .await
            .context("get blaxel sandbox")?;
        decode(response, "get sandbox").await
    }

    /// Returns the most recent non-terminated sandbox for an external id, or
    /// `None` on 404.
    pub async fn get_sandbox_by_external_id(
        &self,
        external_id: &str,
    ) -> anyhow::Result<Option<Sandbox>> {
        let response = self
            .control(
                reqwest::Method::GET,
                &format!(
                    "sandboxes/by-external-id/{}",
                    urlencoding::encode(external_id)
                ),
            )
            .send()
            .await
            .context("get blaxel sandbox by external id")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        decode(response, "get sandbox by external id").await.map(Some)
    }

    pub async fn delete_sandbox(&self, name: &str) -> anyhow::Result<()> {
        let response = self
            .control(
                reqwest::Method::DELETE,
                &format!("sandboxes/{}", urlencoding::encode(name)),
            )
            .send()
            .await
            .context("delete blaxel sandbox")?;
        ensure_ok(response, "delete sandbox").await
    }

    /// Create (or replace) a preview URL for a sandbox port and return the URL.
    pub async fn create_preview(
        &self,
        name: &str,
        port: u16,
        public: bool,
    ) -> anyhow::Result<Option<String>> {
        let preview_name = format!("port-{port}");
        let body = json!({
            "metadata": { "name": preview_name },
            "spec": { "port": port, "public": public }
        });
        let response = self
            .control(
                reqwest::Method::POST,
                &format!("sandboxes/{}/previews?force=true", urlencoding::encode(name)),
            )
            .json(&body)
            .send()
            .await
            .context("create blaxel sandbox preview")?;
        let preview: Preview = decode(response, "create preview").await?;
        Ok(preview.spec.url)
    }

    // ---- per-sandbox API ----------------------------------------------

    /// Wake a standby sandbox by pinging its health endpoint.
    pub async fn wake(&self, endpoint: &str) -> anyhow::Result<()> {
        let response = self
            .sandbox(endpoint, reqwest::Method::GET, "health")
            .send()
            .await
            .context("wake blaxel sandbox")?;
        // Any response means the VM resumed; non-2xx health is non-fatal.
        let _ = response.status();
        Ok(())
    }

    pub async fn exec(
        &self,
        endpoint: &str,
        command: &str,
        working_dir: Option<&str>,
        env: &[(String, String)],
    ) -> anyhow::Result<ProcessResponse> {
        let mut body = json!({
            "command": command,
            "waitForCompletion": true,
        });
        if let Some(dir) = working_dir {
            body["workingDir"] = json!(dir);
        }
        if !env.is_empty() {
            let map: serde_json::Map<String, Value> = env
                .iter()
                .map(|(key, value)| (key.clone(), json!(value)))
                .collect();
            body["env"] = Value::Object(map);
        }
        let response = self
            .sandbox(endpoint, reqwest::Method::POST, "process")
            .json(&body)
            .send()
            .await
            .context("exec blaxel process")?;
        decode(response, "exec process").await
    }

    pub async fn read_file(&self, endpoint: &str, path: &str) -> anyhow::Result<Vec<u8>> {
        let response = self
            .sandbox(
                endpoint,
                reqwest::Method::GET,
                &format!("filesystem/{}", encode_path(path)),
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .context("read blaxel file")?;
        let file: FileWithContent = decode(response, "read file").await?;
        Ok(file.content.into_bytes())
    }

    pub async fn write_file(
        &self,
        endpoint: &str,
        path: &str,
        contents: &[u8],
    ) -> anyhow::Result<()> {
        let content = String::from_utf8(contents.to_vec())
            .context("blaxel runner only supports UTF-8 file writes")?;
        let body = json!({ "content": content });
        let response = self
            .sandbox(
                endpoint,
                reqwest::Method::PUT,
                &format!("filesystem/{}", encode_path(path)),
            )
            .json(&body)
            .send()
            .await
            .context("write blaxel file")?;
        ensure_ok(response, "write file").await
    }

    /// Ensure a directory exists on the sandbox.
    pub async fn make_dir(&self, endpoint: &str, path: &str) -> anyhow::Result<()> {
        let body = json!({ "isDirectory": true });
        let response = self
            .sandbox(
                endpoint,
                reqwest::Method::PUT,
                &format!("filesystem/{}", encode_path(path)),
            )
            .json(&body)
            .send()
            .await
            .context("create blaxel directory")?;
        ensure_ok(response, "create directory").await
    }
}

/// Encode a sandbox path into the `/filesystem/{path}` segment. A leading slash
/// is preserved so absolute paths address the real filesystem root: the Blaxel
/// filesystem and process APIs both treat a leading-slash path as literal from
/// `/`, while a relative path resolves under the sandbox base (`/blaxel`).
/// Dropping the leading slash would silently relocate absolute paths under
/// `/blaxel`, which would not match the working directory used by `exec`.
fn encode_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let encoded = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| urlencoding::encode(segment).into_owned())
        .collect::<Vec<_>>()
        .join("/");
    if absolute {
        format!("/{encoded}")
    } else {
        encoded
    }
}

async fn decode<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    action: &str,
) -> anyhow::Result<T> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("blaxel {action} failed ({status}): {}", error_summary(&body));
    }
    serde_json::from_str(&body)
        .with_context(|| format!("decode blaxel {action} response"))
}

async fn ensure_ok(response: reqwest::Response, action: &str) -> anyhow::Result<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    bail!("blaxel {action} failed ({status}): {}", error_summary(&body));
}

/// Extract a short error message without echoing secrets or large bodies.
fn error_summary(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        if let Some(message) = value
            .get("message")
            .or_else(|| value.get("error"))
            .and_then(Value::as_str)
        {
            return message.to_string();
        }
    }
    body.chars().take(200).collect()
}

/// Poll the control plane until the sandbox reports an endpoint URL, then wake
/// it. Returns the endpoint URL.
pub async fn wait_until_ready(
    client: &BlaxelClient,
    sandbox: Sandbox,
    attempts: u32,
) -> anyhow::Result<(Sandbox, String)> {
    let mut current = sandbox;
    for attempt in 0..attempts.max(1) {
        if current.is_terminated() {
            bail!("blaxel sandbox {} is terminated", current.name());
        }
        if let Some(url) = current.endpoint_url() {
            let url = url.to_string();
            client.wake(&url).await.ok();
            return Ok((current, url));
        }
        if attempt + 1 < attempts.max(1) {
            tokio::time::sleep(Duration::from_millis(250)).await;
            current = client.get_sandbox(current.name()).await?;
        }
    }
    bail!(
        "blaxel sandbox {} did not expose an endpoint url",
        current.name()
    )
}

#[cfg(test)]
mod tests {
    use super::encode_path;

    #[test]
    fn encode_path_preserves_absolute_paths() {
        // Absolute paths keep their leading slash so the filesystem API
        // addresses the real root (matching exec's literal `workingDir`),
        // rather than silently relocating under the sandbox base `/blaxel`.
        assert_eq!(encode_path("/home/user/roder"), "/home/user/roder");
        assert_eq!(encode_path("/home/user/roder/notes.txt"), "/home/user/roder/notes.txt");
    }

    #[test]
    fn encode_path_keeps_relative_paths_relative() {
        assert_eq!(encode_path("notes.txt"), "notes.txt");
        assert_eq!(encode_path("sub/dir/notes.txt"), "sub/dir/notes.txt");
    }

    #[test]
    fn encode_path_escapes_unsafe_segments() {
        assert_eq!(encode_path("/home/a b/c?.txt"), "/home/a%20b/c%3F.txt");
    }
}
