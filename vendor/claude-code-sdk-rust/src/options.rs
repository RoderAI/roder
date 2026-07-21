use crate::types::*;
use std::collections::HashMap;

/// Builder for ClaudeAgentOptions
#[derive(Debug, Default)]
pub struct ClaudeAgentOptionsBuilder {
    options: ClaudeAgentOptions,
}

impl ClaudeAgentOptionsBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tools(mut self, tools: Vec<String>) -> Self {
        self.options.tools = tools;
        self.options.tools_set = true;
        self
    }

    pub fn tools_preset(mut self, preset: ToolsPreset) -> Self {
        self.options.tools_preset = Some(preset);
        self.options.tools_set = false;
        self
    }

    pub fn allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.options.allowed_tools = tools;
        self
    }

    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.options.system_prompt = Some(prompt.into());
        self
    }

    pub fn system_prompt_preset(mut self, preset: SystemPromptPreset) -> Self {
        self.options.system_prompt_preset = Some(preset);
        self
    }

    pub fn system_prompt_file(mut self, file: SystemPromptFile) -> Self {
        self.options.system_prompt_file = Some(file);
        self
    }

    pub fn mcp_server(mut self, name: impl Into<String>, config: MCPServerConfig) -> Self {
        self.options.mcp_servers.insert(name.into(), config);
        self
    }

    pub fn sdk_mcp_server(
        mut self,
        name: impl Into<String>,
        server: crate::mcp::SimpleMCPServer,
    ) -> Self {
        let name = name.into();
        self.options
            .mcp_servers
            .insert(name.clone(), MCPServerConfig::Sdk { name: name.clone() });
        self.options.sdk_mcp_servers.insert(name, server);
        self
    }

    pub fn mcp_servers(mut self, servers: HashMap<String, MCPServerConfig>) -> Self {
        self.options.mcp_servers = servers;
        self.options.mcp_servers_config = None;
        self
    }

    pub fn mcp_servers_config(mut self, config: impl Into<String>) -> Self {
        self.options.mcp_servers.clear();
        self.options.mcp_servers_config = Some(config.into());
        self
    }

    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.options.permission_mode = Some(mode);
        self
    }

    pub fn continue_conversation(mut self, continue_conv: bool) -> Self {
        self.options.continue_conversation = continue_conv;
        self
    }

    pub fn resume(mut self, session_id: impl Into<String>) -> Self {
        self.options.resume = Some(session_id.into());
        self
    }

    pub fn session_id(mut self, session_id: impl Into<String>) -> Self {
        self.options.session_id = Some(session_id.into());
        self
    }

    pub fn fork_session(mut self, fork: bool) -> Self {
        self.options.fork_session = fork;
        self
    }

    pub fn max_turns(mut self, turns: i32) -> Self {
        self.options.max_turns = Some(turns);
        self
    }

    pub fn max_budget_usd(mut self, budget: f64) -> Self {
        self.options.max_budget_usd = Some(budget);
        self
    }

    pub fn task_budget(mut self, budget: TaskBudget) -> Self {
        self.options.task_budget = Some(budget);
        self
    }

    pub fn task_budget_total(mut self, total: i32) -> Self {
        self.options.task_budget = Some(TaskBudget { total });
        self
    }

    pub fn disallowed_tools(mut self, tools: Vec<String>) -> Self {
        self.options.disallowed_tools = tools;
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.options.model = Some(model.into());
        self
    }

    pub fn fallback_model(mut self, model: impl Into<String>) -> Self {
        self.options.fallback_model = Some(model.into());
        self
    }

    pub fn betas(mut self, betas: Vec<SdkBeta>) -> Self {
        self.options.betas = betas;
        self
    }

    pub fn add_beta(mut self, beta: SdkBeta) -> Self {
        self.options.betas.push(beta);
        self
    }

    pub fn permission_prompt_tool_name(mut self, name: impl Into<String>) -> Self {
        self.options.permission_prompt_tool_name = Some(name.into());
        self
    }

    pub fn can_use_tool<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, serde_json::Map<String, serde_json::Value>, ToolPermissionContext) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: std::future::Future<Output = crate::error::Result<PermissionResult>> + Send + 'static,
    {
        self.options.can_use_tool = Some(CanUseToolCallback::new(callback));
        self
    }

    pub fn hook(mut self, event: impl Into<String>, matcher: HookMatcher) -> Self {
        self.options
            .hooks
            .entry(event.into())
            .or_default()
            .push(matcher);
        self
    }

    pub fn hooks(mut self, hooks: HookMap) -> Self {
        self.options.hooks = hooks;
        self
    }

    pub fn agent(mut self, name: impl Into<String>, agent: AgentDefinition) -> Self {
        self.options.agents.insert(name.into(), agent);
        self
    }

    pub fn agents(mut self, agents: HashMap<String, AgentDefinition>) -> Self {
        self.options.agents = agents;
        self
    }

    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.options.cwd = Some(cwd.into());
        self
    }

    pub fn cli_path(mut self, path: impl Into<String>) -> Self {
        self.options.cli_path = Some(path.into());
        self
    }

    pub fn settings(mut self, settings: impl Into<String>) -> Self {
        self.options.settings = Some(settings.into());
        self
    }

    pub fn add_dir(mut self, dir: impl Into<String>) -> Self {
        self.options.add_dirs.push(dir.into());
        self
    }

    pub fn add_dirs(mut self, dirs: Vec<String>) -> Self {
        self.options.add_dirs = dirs;
        self
    }

    pub fn env(mut self, env: HashMap<String, String>) -> Self {
        self.options.env = env;
        self
    }

    pub fn env_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.env.insert(key.into(), value.into());
        self
    }

    pub fn extra_arg(mut self, key: impl Into<String>, value: Option<String>) -> Self {
        self.options.extra_args.insert(key.into(), value);
        self
    }

    pub fn max_buffer_size(mut self, size: usize) -> Self {
        self.options.max_buffer_size = Some(size);
        self
    }

    pub fn user(mut self, user: impl Into<String>) -> Self {
        self.options.user = Some(user.into());
        self
    }

    pub fn include_partial_messages(mut self, include: bool) -> Self {
        self.options.include_partial_messages = include;
        self
    }

    pub fn include_hook_events(mut self, include: bool) -> Self {
        self.options.include_hook_events = include;
        self
    }

    pub fn strict_mcp_config(mut self, strict: bool) -> Self {
        self.options.strict_mcp_config = strict;
        self
    }

    pub fn setting_sources(mut self, sources: Vec<SettingSource>) -> Self {
        self.options.setting_sources = Some(sources);
        self
    }

    pub fn add_setting_source(mut self, source: SettingSource) -> Self {
        self.options
            .setting_sources
            .get_or_insert_with(Vec::new)
            .push(source);
        self
    }

    pub fn skills_all(mut self) -> Self {
        self.options.skills = Some(SkillsConfig::All);
        self
    }

    pub fn skills(mut self, skills: Vec<String>) -> Self {
        self.options.skills = Some(SkillsConfig::Names(skills));
        self
    }

    pub fn sandbox(mut self, sandbox: SandboxSettings) -> Self {
        self.options.sandbox = Some(sandbox);
        self
    }

    pub fn plugin(mut self, plugin: SDKPluginConfig) -> Self {
        self.options.plugins.push(plugin);
        self
    }

    pub fn plugins(mut self, plugins: Vec<SDKPluginConfig>) -> Self {
        self.options.plugins = plugins;
        self
    }

    pub fn max_thinking_tokens(mut self, tokens: i32) -> Self {
        self.options.max_thinking_tokens = Some(tokens);
        self
    }

    pub fn thinking(mut self, thinking: ThinkingConfig) -> Self {
        self.options.thinking = Some(thinking);
        self
    }

    pub fn effort(mut self, effort: EffortLevel) -> Self {
        self.options.effort = Some(effort);
        self
    }

    pub fn output_format(mut self, format: serde_json::Map<String, serde_json::Value>) -> Self {
        self.options.output_format = Some(format);
        self
    }

    pub fn enable_file_checkpointing(mut self, enable: bool) -> Self {
        self.options.enable_file_checkpointing = enable;
        self
    }

    pub fn stderr<F>(mut self, callback: F) -> Self
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        self.options.stderr = Some(StderrCallback::new(callback));
        self
    }

    pub fn session_store<S>(mut self, store: S) -> Self
    where
        S: crate::session_store::SessionStore + 'static,
    {
        self.options.session_store = Some(crate::session_store::SessionStoreHandle::new(store));
        self
    }

    pub fn session_store_handle(mut self, store: crate::session_store::SessionStoreHandle) -> Self {
        self.options.session_store = Some(store);
        self
    }

    pub fn session_store_flush(mut self, mode: SessionStoreFlushMode) -> Self {
        self.options.session_store_flush = mode;
        self
    }

    pub fn load_timeout_ms(mut self, timeout_ms: i32) -> Self {
        self.options.load_timeout_ms = timeout_ms;
        self
    }

    pub fn build(self) -> ClaudeAgentOptions {
        self.options
    }
}

/// Extension trait for ClaudeAgentOptions to add builder method
impl ClaudeAgentOptions {
    pub fn builder() -> ClaudeAgentOptionsBuilder {
        ClaudeAgentOptionsBuilder::new()
    }
}

/// Options for listing sessions
#[derive(Debug, Clone, Default)]
pub struct ListSessionsOptions {
    pub directory: Option<String>,
    pub limit: Option<usize>,
    pub include_worktrees: Option<bool>,
}

impl ListSessionsOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn directory(mut self, dir: impl Into<String>) -> Self {
        self.directory = Some(dir.into());
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn include_worktrees(mut self, include: bool) -> Self {
        self.include_worktrees = Some(include);
        self
    }
}

/// Options for querying session data
#[derive(Debug, Clone, Default)]
pub struct SessionQueryOptions {
    pub directory: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl SessionQueryOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn directory(mut self, dir: impl Into<String>) -> Self {
        self.directory = Some(dir.into());
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }
}

/// Options for session mutations
#[derive(Debug, Clone, Default)]
pub struct SessionMutationOptions {
    pub directory: Option<String>,
}

impl SessionMutationOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn directory(mut self, dir: impl Into<String>) -> Self {
        self.directory = Some(dir.into());
        self
    }
}
