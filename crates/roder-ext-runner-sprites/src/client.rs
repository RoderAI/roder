use std::path::Path;

use anyhow::{Context, bail};
use roder_api::remote_runner::{
    RunnerCommandRequest, RunnerCommandResult, RunnerFileReadRequest, RunnerFileReadResult,
    RunnerFileWriteRequest, RunnerManifest, RunnerPortRequest, RunnerPortResult, RunnerSnapshotRef,
};
use serde::{Deserialize, Serialize};

use crate::config::{PROVIDER_ID, SpritesAppServerConfig, SpritesConfig, redact_text};
use crate::exec_ws::decode_non_tty_stream;
use crate::filesystem::{normalize_workspace_path, target_manifest_path};

#[derive(Debug, Clone)]
pub struct SpritesClient {
    pub(crate) http: reqwest::Client,
    pub config: SpritesConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Sprite {
    pub name: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SpritesCommandResponse {
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    #[serde(default)]
    pub session_id: Option<serde_json::Value>,
    #[serde(default)]
    pub command_id: Option<String>,
    #[serde(default, alias = "code")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SpriteService {
    pub name: String,
    pub cmd: String,
    #[serde(default)]
    pub args: serde_json::Value,
    #[serde(default)]
    pub needs: serde_json::Value,
    #[serde(default)]
    pub http_port: Option<u16>,
    #[serde(default)]
    pub state: Option<SpriteServiceState>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SpriteServiceState {
    pub status: String,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SpritesAppServerDeployment {
    pub service_name: String,
    pub port: u16,
    pub workspace_path: Option<String>,
    pub sprite_url: Option<String>,
    pub health_url: Option<String>,
    pub connect_url: Option<String>,
    pub websocket_url: Option<String>,
    pub proxy_url: String,
    pub token_env: String,
    pub auth_schemes: Vec<String>,
    pub subprotocols: Vec<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SpritesHttpError {
    pub operation: String,
    pub status: reqwest::StatusCode,
    pub body: String,
}

impl std::fmt::Display for SpritesHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} failed with {}: {}",
            self.operation, self.status, self.body
        )
    }
}

impl std::error::Error for SpritesHttpError {}

impl SpritesClient {
    pub fn new(http: reqwest::Client, config: SpritesConfig) -> Self {
        Self { http, config }
    }

    pub async fn ensure_sprite(&self) -> anyhow::Result<Sprite> {
        if let Some(name) = &self.config.sprite_name {
            match self.get_sprite(name).await {
                Ok(sprite) => return Ok(sprite),
                Err(error) => {
                    let message = error.to_string();
                    if !message.contains("404") {
                        return Err(error);
                    }
                }
            }
            return self.create_sprite(name).await;
        }
        let name = self.config.generated_sprite_name();
        self.create_sprite(&name).await
    }

    pub async fn create_sprite(&self, name: &str) -> anyhow::Result<Sprite> {
        let mut body = serde_json::json!({
            "name": name,
            "url_settings": {"auth": self.config.url_auth.as_str()},
        });
        if !self.config.labels.is_empty() {
            body["labels"] = serde_json::json!(self.config.labels);
        }
        if !self.config.metadata.is_null() {
            body["metadata"] = self.config.metadata.clone();
        }
        let response = self
            .http
            .post(self.url("/v1/sprites"))
            .bearer_auth(&self.config.token)
            .json(&body)
            .send()
            .await
            .context("create sprites runner sprite")?;
        self.decode_json(response, "create sprite").await
    }

    pub async fn get_sprite(&self, name: &str) -> anyhow::Result<Sprite> {
        let response = self
            .http
            .get(self.sprite_url(name, ""))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("get sprites runner sprite")?;
        self.decode_json(response, "get sprite").await
    }

    pub async fn delete_sprite(&self, name: &str) -> anyhow::Result<()> {
        let response = self
            .http
            .delete(self.sprite_url(name, ""))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("delete sprites runner sprite")?;
        self.decode_empty(response, "delete sprite").await
    }

    pub async fn materialize_manifest(
        &self,
        sprite_name: &str,
        manifest: &RunnerManifest,
    ) -> anyhow::Result<()> {
        for mount in &manifest.mounts {
            if !matches!(
                mount.intent.kind,
                roder_api::remote_runner::RunnerMountKind::ProviderNative
            ) {
                bail!("sprites runner only supports provider-native mounts");
            }
        }
        for entry in &manifest.entries {
            let source = &entry.source;
            let target = target_manifest_path(&entry.target)?;
            if source.is_file() {
                let contents = tokio::fs::read(source)
                    .await
                    .with_context(|| format!("read manifest source {}", source.display()))?;
                self.write_file_bytes(sprite_name, &target, contents)
                    .await?;
            } else if source.is_dir() {
                self.materialize_manifest_dir(sprite_name, source, &target)
                    .await?;
            } else {
                bail!("runner manifest source {} does not exist", source.display());
            }
        }
        Ok(())
    }

    async fn materialize_manifest_dir(
        &self,
        sprite_name: &str,
        source: &Path,
        target: &str,
    ) -> anyhow::Result<()> {
        let archive = build_manifest_archive(source)?;
        let archive_path = format!(".roder/manifests/{}.tar", uuid::Uuid::new_v4().simple());
        self.write_file_bytes(sprite_name, &archive_path, archive)
            .await?;
        let extract_dir = if target.is_empty() { "." } else { target };
        self.run_install_shell(
            sprite_name,
            format!(
                "mkdir -p {} && tar -xf {} -C {} && rm -f {}",
                shell_quote(extract_dir),
                shell_quote(&archive_path),
                shell_quote(extract_dir),
                shell_quote(&archive_path)
            ),
        )
        .await?;
        Ok(())
    }
}

fn build_manifest_archive(source: &Path) -> anyhow::Result<Vec<u8>> {
    let mut archive = tar::Builder::new(Vec::new());
    {
        let mut pending = vec![source.to_path_buf()];
        while let Some(dir) = pending.pop() {
            let entries = std::fs::read_dir(&dir)
                .with_context(|| format!("read manifest directory {}", dir.display()))?;
            for entry in entries {
                let entry = entry
                    .with_context(|| format!("read manifest directory entry {}", dir.display()))?;
                let path = entry.path();
                let file_type = entry
                    .file_type()
                    .with_context(|| format!("stat manifest source {}", path.display()))?;
                let relative = path.strip_prefix(source).with_context(|| {
                    format!("strip manifest source prefix {}", source.display())
                })?;
                if should_skip_manifest_entry(relative, file_type.is_dir()) {
                    continue;
                }
                if file_type.is_dir() {
                    pending.push(path);
                } else if file_type.is_file() {
                    archive
                        .append_path_with_name(&path, relative)
                        .with_context(|| format!("archive manifest source {}", path.display()))?;
                }
            }
        }
    }
    archive.finish().context("finish manifest archive")?;
    archive.into_inner().context("read manifest archive")
}

impl SpritesClient {
    pub async fn ensure_working_dir(&self, sprite_name: &str) -> anyhow::Result<()> {
        self.write_file_bytes(sprite_name, ".roder-runner-ready", Vec::new())
            .await
    }

    pub async fn deploy_app_server(
        &self,
        sprite: &Sprite,
        app_server: &SpritesAppServerConfig,
    ) -> anyhow::Result<SpritesAppServerDeployment> {
        self.install_roder_binary(&sprite.name, app_server).await?;
        self.ensure_app_server_workspace(&sprite.name, app_server)
            .await?;
        let token = std::env::var(&app_server.auth_token_env).with_context(|| {
            format!(
                "{} is required for sprites app_server bootstrap",
                app_server.auth_token_env
            )
        })?;
        if token.trim().is_empty() {
            bail!("{} is empty", app_server.auth_token_env);
        }
        let mut env = app_server.env.clone();
        env.insert(app_server.auth_token_env.clone(), token);
        for name in &app_server.env_passthrough {
            if env.contains_key(name) {
                continue;
            }
            if let Ok(value) = std::env::var(name)
                && !value.trim().is_empty()
            {
                env.insert(name.clone(), value);
            }
        }
        let service = self
            .put_app_server_service(&sprite.name, app_server, &env)
            .await?;
        let started = if is_service_running(&service) {
            service
        } else {
            let refreshed = if app_server.restart {
                self.restart_service(&sprite.name, &app_server.service_name)
                    .await?;
                self.get_service(&sprite.name, &app_server.service_name)
                    .await
                    .ok()
            } else {
                self.start_service(&sprite.name, &app_server.service_name)
                    .await?;
                self.get_service(&sprite.name, &app_server.service_name)
                    .await
                    .ok()
            };
            refreshed.unwrap_or(service)
        };
        let websocket_url = sprite.url.as_ref().map(|url| {
            let mut url = url.clone();
            if let Some(rest) = url.strip_prefix("https://") {
                url = format!("wss://{rest}");
            } else if let Some(rest) = url.strip_prefix("http://") {
                url = format!("ws://{rest}");
            }
            url
        });
        Ok(SpritesAppServerDeployment {
            service_name: app_server.service_name.clone(),
            port: app_server.port,
            workspace_path: app_server.workspace_path.clone(),
            sprite_url: sprite.url.clone(),
            health_url: sprite
                .url
                .as_ref()
                .map(|url| format!("{}/readyz", url.trim_end_matches('/'))),
            connect_url: websocket_url.clone(),
            websocket_url,
            proxy_url: self
                .sprite_url(&sprite.name, "/proxy")
                .replace("http://", "ws://")
                .replace("https://", "wss://"),
            token_env: app_server.auth_token_env.clone(),
            auth_schemes: vec![
                "authorization_bearer".to_string(),
                "websocket_subprotocol_bearer".to_string(),
            ],
            subprotocols: vec![
                "roder.remote.v1".to_string(),
                format!("bearer.env:{}", app_server.auth_token_env),
            ],
            status: started.state.map(|state| state.status),
        })
    }

    async fn install_roder_binary(
        &self,
        sprite_name: &str,
        app_server: &SpritesAppServerConfig,
    ) -> anyhow::Result<()> {
        if let Some(path) = &app_server.local_binary_path {
            let contents = tokio::fs::read(path)
                .await
                .with_context(|| format!("read local roder binary {path}"))?;
            self.write_file_bytes(sprite_name, &app_server.remote_binary_path, contents)
                .await?;
            self.run_install_shell(
                sprite_name,
                format!("chmod 0755 {}", shell_quote(&app_server.remote_binary_path)),
            )
            .await?;
            return Ok(());
        }
        self.run_install_shell(sprite_name, remote_roder_install_script(app_server))
            .await
    }

    async fn ensure_app_server_workspace(
        &self,
        sprite_name: &str,
        app_server: &SpritesAppServerConfig,
    ) -> anyhow::Result<()> {
        let Some(workspace_path) = app_server.workspace_path.as_deref() else {
            return Ok(());
        };
        let workspace_path = normalize_workspace_path(Path::new(workspace_path))
            .context("normalize sprites app_server workspace_path")?;
        self.write_file_bytes(
            sprite_name,
            &format!("{workspace_path}/.roder-workspace-ready"),
            Vec::new(),
        )
        .await
    }

    async fn run_install_shell(&self, sprite_name: &str, script: String) -> anyhow::Result<()> {
        let result = self
            .run_command(
                sprite_name,
                RunnerCommandRequest {
                    command_id: "sprites-app-server-bootstrap".to_string(),
                    program: "sh".to_string(),
                    args: vec!["-lc".to_string(), script],
                    cwd: None,
                    env: Vec::new(),
                },
            )
            .await?;
        if result.exit_code != Some(0) {
            bail!(
                "sprites app-server bootstrap failed: {}{}",
                result.stdout,
                result.stderr
            );
        }
        Ok(())
    }

    async fn put_app_server_service(
        &self,
        sprite_name: &str,
        app_server: &SpritesAppServerConfig,
        env: &std::collections::BTreeMap<String, String>,
    ) -> anyhow::Result<SpriteService> {
        let mut args = vec![
            "app-server".to_string(),
            "--remote".to_string(),
            "--listen".to_string(),
            format!("ws://0.0.0.0:{}", app_server.port),
            "--auth-token".to_string(),
            format!("env:{}", app_server.auth_token_env),
            "--print-qr=false".to_string(),
            "--config-dir".to_string(),
            app_server.config_dir.clone(),
        ];
        for origin in &app_server.allowed_origins {
            args.push("--allowed-origin".to_string());
            args.push(origin.clone());
        }
        let service_dir = self.app_server_service_dir(app_server)?;
        let service_cmd = self.app_server_service_cmd(app_server);
        let response = self
            .http
            .put(self.service_url(sprite_name, &app_server.service_name, ""))
            .bearer_auth(&self.config.token)
            .json(&serde_json::json!({
                "cmd": service_cmd,
                "args": args,
                "env": env,
                "dir": service_dir,
                "needs": [],
                "http_port": app_server.port,
            }))
            .send()
            .await
            .context("create sprites app-server service")?;
        self.decode_empty(response, "create app-server service")
            .await?;
        self.get_service(sprite_name, &app_server.service_name)
            .await
    }

    fn app_server_service_dir(
        &self,
        app_server: &SpritesAppServerConfig,
    ) -> anyhow::Result<String> {
        let Some(workspace_path) = app_server.workspace_path.as_deref() else {
            return Ok(self.config.working_dir.clone());
        };
        let workspace_path = normalize_workspace_path(Path::new(workspace_path))
            .context("normalize sprites app_server workspace_path")?;
        Ok(format!("{}/{}", self.config.working_dir, workspace_path))
    }

    fn app_server_service_cmd(&self, app_server: &SpritesAppServerConfig) -> String {
        if app_server.remote_binary_path.starts_with('/') {
            app_server.remote_binary_path.clone()
        } else {
            format!(
                "{}/{}",
                self.config.working_dir,
                app_server.remote_binary_path.trim_start_matches("./")
            )
        }
    }

    pub async fn get_service(
        &self,
        sprite_name: &str,
        service_name: &str,
    ) -> anyhow::Result<SpriteService> {
        let response = self
            .http
            .get(self.service_url(sprite_name, service_name, ""))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("get sprites service")?;
        self.decode_json(response, "get service").await
    }

    async fn start_service(&self, sprite_name: &str, service_name: &str) -> anyhow::Result<()> {
        let response = self
            .http
            .post(self.service_url(sprite_name, service_name, "/start?duration=2s"))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("start sprites app-server service")?;
        self.decode_ndjson_for_errors(response, "start app-server service")
            .await?;
        Ok(())
    }

    async fn restart_service(&self, sprite_name: &str, service_name: &str) -> anyhow::Result<()> {
        let response = self
            .http
            .post(self.service_url(sprite_name, service_name, "/restart?duration=2s"))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("restart sprites app-server service")?;
        self.decode_ndjson_for_errors(response, "restart app-server service")
            .await?;
        Ok(())
    }

    pub async fn run_command(
        &self,
        sprite_name: &str,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        let cwd = request
            .cwd
            .as_ref()
            .map(|path| normalize_workspace_path(path))
            .transpose()?
            .map(|path| format!("{}/{}", self.config.working_dir, path))
            .unwrap_or_else(|| self.config.working_dir.clone());
        let response = self
            .http
            .post(self.exec_url(sprite_name, &request, &cwd))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("run sprites runner command")?;
        let command = self.decode_command_response(response).await?;
        Ok(RunnerCommandResult {
            command_id: request.command_id,
            exit_code: command.exit_code,
            stdout: command.stdout,
            stderr: command.stderr,
        })
    }

    pub async fn cancel_command(
        &self,
        sprite_name: &str,
        command_id: &str,
    ) -> anyhow::Result<bool> {
        let response = self
            .http
            .post(self.sprite_url(
                sprite_name,
                &format!("/exec/{}/kill", urlencoding::encode(command_id)),
            ))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("cancel sprites runner command")?;
        self.decode_ndjson_for_errors(response, "cancel command")
            .await?;
        Ok(true)
    }

    pub async fn read_file(
        &self,
        sprite_name: &str,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        let path = normalize_workspace_path(&request.path)?;
        let response = self
            .http
            .get(self.fs_url(
                sprite_name,
                "/fs/read",
                &path,
                &[("workingDir", &self.config.working_dir)],
            ))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("read sprites runner file")?;
        let contents = self.decode_bytes(response, "read file").await?;
        Ok(RunnerFileReadResult {
            path: request.path,
            contents,
        })
    }

    pub async fn write_file(
        &self,
        sprite_name: &str,
        request: RunnerFileWriteRequest,
    ) -> anyhow::Result<()> {
        let path = normalize_workspace_path(&request.path)?;
        self.write_file_bytes(sprite_name, &path, request.contents)
            .await
    }

    async fn write_file_bytes(
        &self,
        sprite_name: &str,
        path: &str,
        contents: Vec<u8>,
    ) -> anyhow::Result<()> {
        let url = self.fs_url(
            sprite_name,
            "/fs/write",
            path,
            &[("workingDir", &self.config.working_dir), ("mkdir", "true")],
        );
        const MAX_FILE_WRITE_ATTEMPTS: u32 = 60;
        for attempt in 0..MAX_FILE_WRITE_ATTEMPTS {
            let response = self
                .http
                .put(&url)
                .bearer_auth(&self.config.token)
                .body(contents.clone())
                .send()
                .await
                .with_context(|| format!("write sprites runner file {path}"))?;
            let status = response.status();
            let body = response.bytes().await?;
            if status.is_success() {
                return Ok(());
            }
            if !is_retryable_file_write_status(status) || attempt + 1 == MAX_FILE_WRITE_ATTEMPTS {
                return Err(SpritesHttpError {
                    operation: format!("write file {path}"),
                    status,
                    body: redact_text(&String::from_utf8_lossy(&body), &self.config.token),
                }
                .into());
            }
            let delay_ms = 250_u64 * u64::from((attempt + 1).min(8));
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        Ok(())
    }

    pub async fn expose_port(
        &self,
        sprite: &Sprite,
        request: RunnerPortRequest,
    ) -> anyhow::Result<RunnerPortResult> {
        Ok(RunnerPortResult {
            port: request.port,
            url: sprite.url.as_ref().map(|url| {
                if self.config.url_auth.as_str() == "public" {
                    url.clone()
                } else {
                    format!("{url}#port-{}", request.port)
                }
            }),
        })
    }

    pub async fn snapshot(&self, sprite_name: &str) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        let response = self
            .http
            .post(self.sprite_url(sprite_name, "/checkpoint"))
            .bearer_auth(&self.config.token)
            .json(&serde_json::json!({"comment": "Roder runner snapshot"}))
            .send()
            .await
            .context("snapshot sprites runner")?;
        let body = self
            .decode_ndjson_for_errors(response, "create checkpoint")
            .await?;
        let snapshot_id = checkpoint_id_from_ndjson(&body).unwrap_or_else(|| "latest".to_string());
        Ok(Some(RunnerSnapshotRef {
            provider_id: PROVIDER_ID.to_string(),
            snapshot_id,
            metadata: serde_json::json!({"sprite_name": sprite_name}),
        }))
    }

    pub async fn export_artifact(
        &self,
        sprite_name: &str,
        request: roder_api::remote_runner::RunnerArtifactExportRequest,
    ) -> anyhow::Result<roder_api::remote_runner::RunnerArtifactExportResult> {
        if request.recursive {
            bail!("sprites runner artifact export currently supports files only");
        }
        let read = self
            .read_file(
                sprite_name,
                RunnerFileReadRequest {
                    path: request.path.clone(),
                },
            )
            .await?;
        Ok(roder_api::remote_runner::RunnerArtifactExportResult {
            path: request.path,
            artifact_id: format!("sprites-inline-{}", read.contents.len()),
            url: None,
            metadata: serde_json::json!({
                "provider": PROVIDER_ID,
                "size": read.contents.len(),
                "inline": true
            }),
        })
    }

    pub async fn put_policy(
        &self,
        sprite_name: &str,
        path: &str,
        value: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let response = self
            .http
            .put(self.sprite_url(sprite_name, path))
            .bearer_auth(&self.config.token)
            .json(value)
            .send()
            .await
            .with_context(|| format!("apply sprites policy {path}"))?;
        self.decode_empty(response, "apply policy").await
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.config.base_url, path)
    }

    pub(crate) fn sprite_url(&self, sprite_name: &str, path: &str) -> String {
        format!(
            "{}/v1/sprites/{}{}",
            self.config.base_url,
            urlencoding::encode(sprite_name),
            path
        )
    }

    fn service_url(&self, sprite_name: &str, service_name: &str, path: &str) -> String {
        format!(
            "{}/v1/sprites/{}/services/{}{}",
            self.config.base_url,
            urlencoding::encode(sprite_name),
            urlencoding::encode(service_name),
            path
        )
    }

    fn exec_url(&self, sprite_name: &str, request: &RunnerCommandRequest, cwd: &str) -> String {
        let mut url = self.sprite_url(sprite_name, "/exec");
        let mut pairs = vec![
            ("path".to_string(), request.program.clone()),
            ("dir".to_string(), cwd.to_string()),
        ];
        pairs.push(("cmd".to_string(), request.program.clone()));
        for arg in &request.args {
            pairs.push(("cmd".to_string(), arg.clone()));
        }
        for (key, value) in &request.env {
            pairs.push(("env".to_string(), format!("{key}={value}")));
        }
        let query = pairs
            .into_iter()
            .map(|(key, value)| format!("{key}={}", urlencoding::encode(&value)))
            .collect::<Vec<_>>()
            .join("&");
        url.push('?');
        url.push_str(&query);
        url
    }

    pub(crate) fn fs_url(
        &self,
        sprite_name: &str,
        endpoint: &str,
        path: &str,
        extra: &[(&str, &str)],
    ) -> String {
        let mut url = self.sprite_url(sprite_name, endpoint);
        let mut pairs = vec![("path".to_string(), path.to_string())];
        pairs.extend(
            extra
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string())),
        );
        let query = pairs
            .into_iter()
            .map(|(key, value)| format!("{key}={}", urlencoding::encode(&value)))
            .collect::<Vec<_>>()
            .join("&");
        url.push('?');
        url.push_str(&query);
        url
    }

    pub(crate) async fn decode_json<T: for<'de> Deserialize<'de>>(
        &self,
        response: reqwest::Response,
        operation: &str,
    ) -> anyhow::Result<T> {
        let status = response.status();
        let body = response.bytes().await?;
        if !status.is_success() {
            return Err(SpritesHttpError {
                operation: operation.to_string(),
                status,
                body: redact_text(&String::from_utf8_lossy(&body), &self.config.token),
            }
            .into());
        }
        serde_json::from_slice(&body)
            .with_context(|| format!("decode sprites {operation} response"))
    }

    async fn decode_command_response(
        &self,
        response: reqwest::Response,
    ) -> anyhow::Result<SpritesCommandResponse> {
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.bytes().await?;
        if !status.is_success() {
            return Err(SpritesHttpError {
                operation: "run command".to_string(),
                status,
                body: redact_text(&String::from_utf8_lossy(&body), &self.config.token),
            }
            .into());
        }
        if content_type.starts_with("application/json") {
            return serde_json::from_slice(&body)
                .with_context(|| "decode sprites run command response");
        }
        let output = decode_non_tty_stream(&body);
        Ok(SpritesCommandResponse {
            id: None,
            session_id: None,
            command_id: None,
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    pub(crate) async fn decode_bytes(
        &self,
        response: reqwest::Response,
        operation: &str,
    ) -> anyhow::Result<Vec<u8>> {
        let status = response.status();
        let body = response.bytes().await?;
        if !status.is_success() {
            return Err(SpritesHttpError {
                operation: operation.to_string(),
                status,
                body: redact_text(&String::from_utf8_lossy(&body), &self.config.token),
            }
            .into());
        }
        Ok(body.to_vec())
    }

    pub(crate) async fn decode_empty(
        &self,
        response: reqwest::Response,
        operation: &str,
    ) -> anyhow::Result<()> {
        let status = response.status();
        let body = response.bytes().await?;
        if !status.is_success() {
            return Err(SpritesHttpError {
                operation: operation.to_string(),
                status,
                body: redact_text(&String::from_utf8_lossy(&body), &self.config.token),
            }
            .into());
        }
        Ok(())
    }

    pub(crate) async fn decode_ndjson_for_errors(
        &self,
        response: reqwest::Response,
        operation: &str,
    ) -> anyhow::Result<String> {
        let status = response.status();
        let body = response.bytes().await?;
        let text = String::from_utf8_lossy(&body).to_string();
        if !status.is_success() {
            return Err(SpritesHttpError {
                operation: operation.to_string(),
                status,
                body: redact_text(&text, &self.config.token),
            }
            .into());
        }
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                if value.get("type").and_then(serde_json::Value::as_str) == Some("error") {
                    bail!(
                        "{} failed: {}",
                        operation,
                        redact_text(
                            value
                                .get("error")
                                .or_else(|| value.get("message"))
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("unknown sprites error"),
                            &self.config.token
                        )
                    );
                }
            }
        }
        Ok(text)
    }
}

fn should_skip_manifest_entry(relative: &Path, is_dir: bool) -> bool {
    if !is_dir {
        return false;
    }
    let components = relative
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => Some(name.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if components.first().map(|name| name.as_ref()) == Some("roadmap")
        && components.get(1).map(|name| name.as_ref()) == Some("assets")
    {
        return true;
    }
    let Some(first) = relative.components().next() else {
        return false;
    };
    let std::path::Component::Normal(name) = first else {
        return false;
    };
    matches!(
        name.to_string_lossy().as_ref(),
        ".git"
            | ".hg"
            | ".svn"
            | ".roder"
            | ".codex"
            | "target"
            | "node_modules"
            | ".next"
            | "dist"
            | "build"
    )
}

fn is_service_running(service: &SpriteService) -> bool {
    service.state.as_ref().map(|state| state.status.as_str()) == Some("running")
}

fn is_retryable_file_write_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::NOT_FOUND
            | reqwest::StatusCode::BAD_GATEWAY
            | reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::GATEWAY_TIMEOUT
    )
}

fn checkpoint_id_from_ndjson(body: &str) -> Option<String> {
    for line in body.lines().rev() {
        let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
        for key in ["checkpoint_id", "id"] {
            if let Some(id) = value.get(key).and_then(serde_json::Value::as_str) {
                return Some(id.to_string());
            }
        }
        if value.get("type").and_then(serde_json::Value::as_str) == Some("complete") {
            if let Some(data) = value.get("data").and_then(serde_json::Value::as_str) {
                if let Some(id) = data.split_whitespace().find(|part| part.starts_with('v')) {
                    return Some(
                        id.trim_matches(|ch: char| !ch.is_alphanumeric())
                            .to_string(),
                    );
                }
            }
        }
    }
    None
}

fn remote_roder_install_script(app_server: &SpritesAppServerConfig) -> String {
    let base_url = shell_quote(app_server.download_base_url.trim_end_matches('/'));
    let binary_name = shell_quote(&app_server.binary_name);
    let remote_path = shell_quote(&app_server.remote_binary_path);
    format!(
        r#"set -eu
base_url={base_url}
binary_name={binary_name}
remote_path={remote_path}
arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) arch_part="x86_64" ;;
  aarch64|arm64) arch_part="aarch64" ;;
  *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
esac
target="${{arch_part}}-unknown-linux-gnu"
artifact="${{binary_name}}-${{target}}"
mkdir -p "$(dirname "$remote_path")"
tmp_dir="$(mktemp -d)"
cleanup() {{
  rm -rf "$tmp_dir"
}}
trap cleanup EXIT INT TERM
download() {{
  url="$1"
  out="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 --retry-delay 1 "$url" -o "$out"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$url" -O "$out"
  else
    echo "curl or wget is required" >&2
    exit 1
  fi
}}
download "${{base_url}}/${{artifact}}" "${{tmp_dir}}/${{artifact}}"
download "${{base_url}}/${{artifact}}.sha256" "${{tmp_dir}}/${{artifact}}.sha256"
(
  cd "$tmp_dir"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "${{artifact}}.sha256"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "${{artifact}}.sha256"
  else
    echo "sha256sum or shasum is required" >&2
    exit 1
  fi
)
mv "${{tmp_dir}}/${{artifact}}" "$remote_path"
chmod 0755 "$remote_path"
"#
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
