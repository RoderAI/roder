//! What every group of fake-server tests shares: launching the fake behind a
//! `BrowserUseServer`, the tool registry, and running one tool call.

use std::sync::Arc;

use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolRegistry};
use roder_ext_browser_use::{
    BrowserUseConfig, BrowserUseExtension, BrowserUseServer, server_command,
};
use serde_json::Value;

use crate::fake::{DROP_TOOL_ENV, FAKE_ENV};

pub const OPENAI_KEY: &str = "sk-fake-openai-key-123456";

pub fn fake_launch(config: BrowserUseConfig, drop_tool: Option<&str>) -> Arc<BrowserUseServer> {
    let exe = std::env::current_exe().unwrap();
    let drop_tool = drop_tool.map(str::to_string);
    let package = config.package.clone();
    Arc::new(BrowserUseServer::with_launch(
        package,
        Arc::new(move || {
            let mut parent: Vec<(String, String)> = std::env::vars().collect();
            parent.push((
                "GITHUB_TOKEN".into(),
                "ghp-must-not-reach-the-server".into(),
            ));
            let mut spec = server_command(&config, exe.clone(), parent);
            spec.args = vec![
                "fake_server_entry".into(),
                "--exact".into(),
                "--nocapture".into(),
                "--test-threads=1".into(),
                "--quiet".into(),
            ];
            spec.env.insert(FAKE_ENV.into(), "1".into());
            if let Some(tool) = &drop_tool {
                spec.env.insert(DROP_TOOL_ENV.into(), tool.clone());
            }
            Ok(spec)
        }),
    ))
}

pub fn keyed_config() -> BrowserUseConfig {
    BrowserUseConfig {
        openai_api_key: Some(OPENAI_KEY.into()),
        ..BrowserUseConfig::default()
    }
}

pub fn registry_for(server: Arc<BrowserUseServer>, has_llm_key: bool) -> ToolRegistry {
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(BrowserUseExtension::with_server(server, has_llm_key))
        .unwrap();
    let extensions = builder.build().unwrap();
    let mut registry = ToolRegistry::default();
    for contributor in &extensions.tools {
        contributor.contribute(&mut registry).unwrap();
    }
    registry
}

pub async fn run(registry: &ToolRegistry, name: &str, args: Value) -> roder_api::tools::ToolResult {
    run_thread(registry, "thread", name, args).await
}

pub async fn run_thread(
    registry: &ToolRegistry,
    thread: &str,
    name: &str,
    args: Value,
) -> roder_api::tools::ToolResult {
    let tool = registry
        .get(name)
        .unwrap_or_else(|| panic!("{name} registered"));
    tool.execute(
        ToolExecutionContext::new(thread, "turn", PolicyMode::Default),
        ToolCall {
            id: "call".into(),
            name: name.into(),
            raw_arguments: args.to_string(),
            arguments: args,
            thread_id: thread.into(),
            turn_id: "turn".into(),
        },
    )
    .await
    .unwrap()
}

pub fn state_json(text: &str) -> Value {
    let body = text.split("\n---\n").nth(1).expect("untrusted envelope");
    // The first result from a replacement browser has a notice before the JSON.
    let json = &body[body.find('{').expect("state json")..];
    serde_json::from_str(json).expect("state json")
}

#[cfg(unix)]
pub fn alive(pid: u64) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
