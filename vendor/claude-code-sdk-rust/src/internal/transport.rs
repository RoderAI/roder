use async_trait::async_trait;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::error::{CLIConnectionError, CLINotFoundError, ClaudeSDKError, ProcessError, Result};
use crate::internal::stdout_decoder::StdoutDecoder;
use crate::types::ClaudeAgentOptions;

const DEFAULT_ENTRY_POINT: &str = "sdk-rust";
const DEFAULT_MAX_BUFFER_SIZE: usize = 1024 * 1024;

#[async_trait]
pub trait Transport: Send + Sync {
    async fn connect(&mut self) -> Result<()>;
    async fn write(&mut self, data: &[u8]) -> Result<()>;
    async fn close_input(&mut self) -> Result<()>;
    async fn read(&mut self) -> Result<Option<Vec<u8>>>;
    async fn close(&mut self) -> Result<()>;
}

#[derive(Debug)]
pub struct SubprocessCLITransport {
    options: TransportOptions,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout_reader: Option<BufReader<ChildStdout>>,
    stdout_decoder: StdoutDecoder,
    stderr: Arc<Mutex<String>>,
}

#[derive(Debug, Clone)]
pub struct TransportOptions {
    pub tools: Vec<String>,
    pub tools_set: bool,
    pub tools_preset: Option<crate::types::ToolsPreset>,
    pub allowed_tools: Vec<String>,
    pub system_prompt: Option<String>,
    pub system_prompt_preset: Option<crate::types::SystemPromptPreset>,
    pub system_prompt_file: Option<crate::types::SystemPromptFile>,
    pub mcp_servers: std::collections::HashMap<String, crate::types::MCPServerConfig>,
    pub mcp_servers_config: Option<String>,
    pub permission_mode: Option<crate::types::PermissionMode>,
    pub continue_conversation: bool,
    pub resume: Option<String>,
    pub session_id: Option<String>,
    pub fork_session: bool,
    pub max_turns: Option<i32>,
    pub max_budget_usd: Option<f64>,
    pub task_budget: Option<crate::types::TaskBudget>,
    pub disallowed_tools: Vec<String>,
    pub model: Option<String>,
    pub fallback_model: Option<String>,
    pub betas: Vec<crate::types::SdkBeta>,
    pub permission_prompt_tool_name: Option<String>,
    pub cwd: Option<String>,
    pub cli_path: Option<String>,
    pub settings: Option<String>,
    pub add_dirs: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    pub extra_args: std::collections::HashMap<String, Option<String>>,
    pub max_buffer_size: Option<usize>,
    pub user: Option<String>,
    pub include_partial_messages: bool,
    pub include_hook_events: bool,
    pub strict_mcp_config: bool,
    pub setting_sources: Option<Vec<crate::types::SettingSource>>,
    pub skills: Option<crate::types::SkillsConfig>,
    pub sandbox: Option<crate::types::SandboxSettings>,
    pub plugins: Vec<crate::types::SDKPluginConfig>,
    pub max_thinking_tokens: Option<i32>,
    pub thinking: Option<crate::types::ThinkingConfig>,
    pub effort: Option<crate::types::EffortLevel>,
    pub output_format: Option<serde_json::Map<String, serde_json::Value>>,
    pub enable_file_checkpointing: bool,
    pub stderr: Option<crate::types::StderrCallback>,
    pub can_use_tool: Option<crate::types::CanUseToolCallback>,
    pub sdk_mcp_servers: std::collections::HashMap<String, crate::mcp::SimpleMCPServer>,
    pub session_store_enabled: bool,
}

impl From<&ClaudeAgentOptions> for TransportOptions {
    fn from(opts: &ClaudeAgentOptions) -> Self {
        Self {
            tools: opts.tools.clone(),
            tools_set: opts.tools_set,
            tools_preset: opts.tools_preset.clone(),
            allowed_tools: opts.allowed_tools.clone(),
            system_prompt: opts.system_prompt.clone(),
            system_prompt_preset: opts.system_prompt_preset.clone(),
            system_prompt_file: opts.system_prompt_file.clone(),
            mcp_servers: opts.mcp_servers.clone(),
            mcp_servers_config: opts.mcp_servers_config.clone(),
            permission_mode: opts.permission_mode,
            continue_conversation: opts.continue_conversation,
            resume: opts.resume.clone(),
            session_id: opts.session_id.clone(),
            fork_session: opts.fork_session,
            max_turns: opts.max_turns,
            max_budget_usd: opts.max_budget_usd,
            task_budget: opts.task_budget.clone(),
            disallowed_tools: opts.disallowed_tools.clone(),
            model: opts.model.clone(),
            fallback_model: opts.fallback_model.clone(),
            betas: opts.betas.clone(),
            permission_prompt_tool_name: opts
                .permission_prompt_tool_name
                .clone()
                .or_else(|| opts.can_use_tool.as_ref().map(|_| "stdio".to_string())),
            cwd: opts.cwd.clone(),
            cli_path: opts.cli_path.clone(),
            settings: opts.settings.clone(),
            add_dirs: opts.add_dirs.clone(),
            env: opts.env.clone(),
            extra_args: opts.extra_args.clone(),
            max_buffer_size: opts.max_buffer_size,
            user: opts.user.clone(),
            include_partial_messages: opts.include_partial_messages,
            include_hook_events: opts.include_hook_events,
            strict_mcp_config: opts.strict_mcp_config,
            setting_sources: opts.setting_sources.clone(),
            skills: opts.skills.clone(),
            sandbox: opts.sandbox.clone(),
            plugins: opts.plugins.clone(),
            max_thinking_tokens: opts.max_thinking_tokens,
            thinking: opts.thinking.clone(),
            effort: opts.effort.clone(),
            output_format: opts.output_format.clone(),
            enable_file_checkpointing: opts.enable_file_checkpointing,
            stderr: opts.stderr.clone(),
            can_use_tool: opts.can_use_tool.clone(),
            sdk_mcp_servers: opts.sdk_mcp_servers.clone(),
            session_store_enabled: opts.session_store.is_some(),
        }
    }
}

impl SubprocessCLITransport {
    pub fn new(options: TransportOptions) -> Self {
        let max_buffer_size = options.max_buffer_size.unwrap_or(DEFAULT_MAX_BUFFER_SIZE);
        Self {
            options,
            child: None,
            stdin: None,
            stdout_reader: None,
            stdout_decoder: StdoutDecoder::new(max_buffer_size),
            stderr: Arc::new(Mutex::new(String::new())),
        }
    }

    fn resolve_cli_path(&self) -> Result<String> {
        crate::internal::cli_discovery::find_cli_path(self.options.cli_path.as_deref())
    }

    fn build_args(&self) -> Result<Vec<String>> {
        crate::internal::cli_args::build_cli_args(&self.options)
    }

    fn build_env(&self) -> std::collections::HashMap<String, String> {
        build_process_env(std::env::vars(), &self.options)
    }
    async fn finish_read(&mut self) -> Result<Option<Vec<u8>>> {
        if let Some(ref mut child) = self.child {
            match child.wait().await {
                Ok(status) => {
                    if !status.success() {
                        let stderr = self.stderr.lock().await.clone();
                        return Err(ProcessError::new(
                            "Claude Code process exited with error",
                            status.code(),
                            stderr,
                        )
                        .into());
                    }
                }
                Err(e) => {
                    return Err(CLIConnectionError::new(format!(
                        "failed to wait for process: {}",
                        e
                    ))
                    .into());
                }
            }
        }
        Ok(None)
    }
}

fn build_process_env<I>(inherited: I, options: &TransportOptions) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut env = inherited
        .into_iter()
        .filter(|(key, _)| key != "CLAUDECODE")
        .collect::<HashMap<_, _>>();

    env.insert(
        "CLAUDE_CODE_ENTRYPOINT".to_string(),
        DEFAULT_ENTRY_POINT.to_string(),
    );

    for (key, value) in &options.env {
        env.insert(key.clone(), value.clone());
    }

    env.insert(
        "CLAUDE_AGENT_SDK_VERSION".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    );

    apply_otel_trace_context(&mut env, &options.env, active_otel_trace_context());

    if options.enable_file_checkpointing {
        env.insert(
            "CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING".to_string(),
            "true".to_string(),
        );
    }

    if let Some(ref cwd) = options.cwd {
        env.insert("PWD".to_string(), cwd.clone());
    }

    env
}

fn apply_otel_trace_context(
    env: &mut HashMap<String, String>,
    explicit_env: &HashMap<String, String>,
    carrier: HashMap<String, String>,
) {
    if !carrier.contains_key("traceparent") {
        return;
    }

    for key in ["TRACEPARENT", "TRACESTATE"] {
        if !explicit_env.contains_key(key) {
            env.remove(key);
        }
    }

    for (key, value) in carrier {
        let env_key = key.to_ascii_uppercase();
        if !explicit_env.contains_key(&env_key) {
            env.insert(env_key, value);
        }
    }
}

#[cfg(feature = "otel")]
fn active_otel_trace_context() -> HashMap<String, String> {
    let mut carrier = HashMap::new();
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject(&mut carrier);
    });
    carrier
}

#[cfg(not(feature = "otel"))]
fn active_otel_trace_context() -> HashMap<String, String> {
    HashMap::new()
}

#[async_trait]
impl Transport for SubprocessCLITransport {
    async fn connect(&mut self) -> Result<()> {
        if self.child.is_some() {
            return Ok(());
        }

        let cli_path = self.resolve_cli_path()?;
        if std::env::var_os("CLAUDE_AGENT_SDK_SKIP_VERSION_CHECK").is_none() {
            let _ = crate::internal::cli_discovery::check_cli_version(&cli_path).await;
        }

        if let Some(ref cwd) = self.options.cwd {
            if !tokio::fs::metadata(cwd)
                .await
                .map(|m| m.is_dir())
                .unwrap_or(false)
            {
                return Err(CLIConnectionError::new(format!(
                    "working directory does not exist: {}",
                    cwd
                ))
                .into());
            }
        }

        let args = self.build_args()?;
        let env = self.build_env();

        let mut cmd = Command::new(&cli_path);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(ref cwd) = self.options.cwd {
            cmd.current_dir(cwd);
        }

        for (key, value) in &env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ClaudeSDKError::CLINotFound(CLINotFoundError::new(
                    "Claude Code not found",
                    cli_path,
                ))
            } else {
                CLIConnectionError::new(format!("failed to start Claude Code: {}", e)).into()
            }
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| CLIConnectionError::new("failed to open CLI stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CLIConnectionError::new("failed to open CLI stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CLIConnectionError::new("failed to open CLI stderr"))?;

        let stderr_arc = self.stderr.clone();
        let stderr_callback = self.options.stderr.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 {
                    break;
                }
                let mut stderr_guard = stderr_arc.lock().await;
                stderr_guard.push_str(&line);
                if let Some(callback) = &stderr_callback {
                    callback.call(line.clone());
                }
                line.clear();
            }
        });

        self.child = Some(child);
        self.stdin = Some(stdin);
        self.stdout_reader = Some(BufReader::new(stdout));

        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| CLIConnectionError::new("transport is not connected"))?;

        stdin
            .write_all(data)
            .await
            .map_err(|e| CLIConnectionError::new(format!("failed to write to stdin: {}", e)))?;
        stdin
            .flush()
            .await
            .map_err(|e| CLIConnectionError::new(format!("failed to flush stdin: {}", e)))?;

        Ok(())
    }

    async fn close_input(&mut self) -> Result<()> {
        if let Some(mut stdin) = self.stdin.take() {
            stdin
                .shutdown()
                .await
                .map_err(|e| CLIConnectionError::new(format!("failed to close stdin: {}", e)))?;
        }
        Ok(())
    }

    async fn read(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            if let Some(data) = self.stdout_decoder.next() {
                return Ok(Some(data));
            }

            let mut chunk = [0u8; 8192];
            let read_result = {
                let reader = self
                    .stdout_reader
                    .as_mut()
                    .ok_or_else(|| CLIConnectionError::new("transport is not connected"))?;
                reader.read(&mut chunk).await
            };

            match read_result {
                Ok(0) => {
                    self.stdout_decoder.finish()?;
                    if let Some(data) = self.stdout_decoder.next() {
                        return Ok(Some(data));
                    }
                    return self.finish_read().await;
                }
                Ok(n) => self
                    .stdout_decoder
                    .push(std::str::from_utf8(&chunk[..n]).map_err(|e| {
                        CLIConnectionError::new(format!("stdout was not valid UTF-8: {}", e))
                    })?)?,
                Err(e) => {
                    return Err(
                        CLIConnectionError::new(format!("failed reading stdout: {}", e)).into(),
                    )
                }
            }
        }
    }

    async fn close(&mut self) -> Result<()> {
        let _ = self.close_input().await;

        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_env_matches_python_sdk_subprocess_defaults() {
        let options = crate::types::ClaudeAgentOptions::builder()
            .env_var("CLAUDE_CODE_ENTRYPOINT", "custom-entrypoint")
            .env_var("TRACEPARENT", "explicit-trace")
            .cwd("/tmp/project")
            .enable_file_checkpointing(true)
            .build();
        let transport_options = TransportOptions::from(&options);

        let env = build_process_env(
            [
                ("CLAUDECODE".to_string(), "1".to_string()),
                ("PATH".to_string(), "/bin".to_string()),
                ("TRACEPARENT".to_string(), "ambient-trace".to_string()),
            ],
            &transport_options,
        );

        assert_eq!(env.get("CLAUDECODE"), None);
        assert_eq!(
            env.get("CLAUDE_CODE_ENTRYPOINT").map(String::as_str),
            Some("custom-entrypoint")
        );
        assert_eq!(
            env.get("CLAUDE_AGENT_SDK_VERSION").map(String::as_str),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            env.get("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING")
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(env.get("PWD").map(String::as_str), Some("/tmp/project"));
        assert_eq!(
            env.get("TRACEPARENT").map(String::as_str),
            Some("explicit-trace")
        );
    }

    #[test]
    fn process_env_injects_active_otel_context_like_python_sdk() {
        let options = crate::types::ClaudeAgentOptions::builder().build();
        let transport_options = TransportOptions::from(&options);
        let mut env = build_process_env(
            [
                (
                    "TRACEPARENT".to_string(),
                    "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string(),
                ),
                ("TRACESTATE".to_string(), "vendor=stale".to_string()),
            ],
            &transport_options,
        );

        apply_otel_trace_context(
            &mut env,
            &transport_options.env,
            HashMap::from([
                (
                    "traceparent".to_string(),
                    "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".to_string(),
                ),
                ("tracestate".to_string(), "vendor=value".to_string()),
            ]),
        );

        assert_eq!(
            env.get("TRACEPARENT").map(String::as_str),
            Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")
        );
        assert_eq!(
            env.get("TRACESTATE").map(String::as_str),
            Some("vendor=value")
        );
    }

    #[test]
    fn process_env_preserves_explicit_traceparent_over_otel_context() {
        let options = crate::types::ClaudeAgentOptions::builder()
            .env_var("TRACEPARENT", "custom")
            .build();
        let transport_options = TransportOptions::from(&options);
        let mut env = build_process_env(
            [("TRACEPARENT".to_string(), "ambient".to_string())],
            &transport_options,
        );

        apply_otel_trace_context(
            &mut env,
            &transport_options.env,
            HashMap::from([("traceparent".to_string(), "active".to_string())]),
        );

        assert_eq!(env.get("TRACEPARENT").map(String::as_str), Some("custom"));
    }

    #[test]
    fn process_env_preserves_inherited_w3c_env_without_active_otel_span() {
        let options = crate::types::ClaudeAgentOptions::builder().build();
        let transport_options = TransportOptions::from(&options);
        let mut env = build_process_env(
            [
                ("TRACEPARENT".to_string(), "ambient".to_string()),
                ("TRACESTATE".to_string(), "vendor=abc".to_string()),
            ],
            &transport_options,
        );

        apply_otel_trace_context(
            &mut env,
            &transport_options.env,
            HashMap::from([("baggage".to_string(), "user.id=123".to_string())]),
        );

        assert_eq!(env.get("TRACEPARENT").map(String::as_str), Some("ambient"));
        assert_eq!(
            env.get("TRACESTATE").map(String::as_str),
            Some("vendor=abc")
        );
    }

    #[tokio::test]
    async fn subprocess_stderr_callback_receives_lines() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        let dir =
            std::env::temp_dir().join(format!("claude-rust-stderr-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("claude");
        let mut file = std::fs::File::create(&script).unwrap();
        writeln!(
            file,
            r#"#!/bin/sh
if [ "$1" = "-v" ]; then
  printf '2.0.0 (Claude Code)\n'
  exit 0
fi
printf 'diagnostic line\n' >&2
printf '{{"type":"result","subtype":"success","duration_ms":1,"duration_api_ms":1,"is_error":false,"num_turns":1,"session_id":"s"}}\n'
"#
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&script).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&script, permissions).unwrap();
        }

        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let captured = lines.clone();
        let options = crate::types::ClaudeAgentOptions::builder()
            .cli_path(script.to_string_lossy().to_string())
            .stderr(move |line| captured.lock().unwrap().push(line))
            .build();
        let mut transport = SubprocessCLITransport::new(TransportOptions::from(&options));

        transport.connect().await.unwrap();
        let message = transport.read().await.unwrap().expect("result");
        let value: serde_json::Value = serde_json::from_slice(&message).unwrap();
        assert_eq!(value["type"], "result");

        for _ in 0..20 {
            if lines
                .lock()
                .unwrap()
                .iter()
                .any(|line| line == "diagnostic line\n")
            {
                let _ = transport.close().await;
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("stderr callback did not receive diagnostic line");
    }
}
