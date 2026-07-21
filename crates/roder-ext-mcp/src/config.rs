use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Credential source permitted for MCP tool execution.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpToolCallAuthMode {
    /// Use a thread-scoped token when present, otherwise use the server's
    /// configured credential.
    #[default]
    ConfiguredCredential,
    /// Require a token registered for the executing thread. The configured
    /// credential remains available for discovery only.
    ThreadScopedRequired,
}

impl McpToolCallAuthMode {
    fn is_default(&self) -> bool {
        *self == Self::ConfiguredCredential
    }
}

/// One MCP server reachable over streamable HTTP.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    /// Short identifier used in tool names (`mcp__<name>__<tool>`).
    pub name: String,
    /// Full URL of the MCP endpoint, e.g. `https://vex.sc/api/v1/mcp`.
    pub url: String,
    /// Bearer token sent as `Authorization: Bearer <token>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    /// Environment variable to read the bearer token from when `auth_token`
    /// is unset. Preferred for generated distributions so secrets stay out of
    /// config files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token_env: Option<String>,
    /// Credential policy for `tools/call`. Discovery always uses the
    /// configured credential.
    #[serde(default, skip_serializing_if = "McpToolCallAuthMode::is_default")]
    pub tool_call_auth_mode: McpToolCallAuthMode,
    /// Extra headers added to every request.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// When non-empty, only these remote tool names are exposed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_tools: Vec<String>,
}

impl McpServerConfig {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            auth_token: None,
            auth_token_env: None,
            tool_call_auth_mode: McpToolCallAuthMode::default(),
            headers: BTreeMap::new(),
            enabled_tools: Vec::new(),
        }
    }

    pub fn with_auth_token(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }

    pub fn with_auth_token_env(mut self, var: impl Into<String>) -> Self {
        self.auth_token_env = Some(var.into());
        self
    }

    pub fn with_tool_call_auth_mode(mut self, mode: McpToolCallAuthMode) -> Self {
        self.tool_call_auth_mode = mode;
        self
    }

    /// Resolves the bearer token from the literal value or the environment.
    pub fn resolve_auth_token(&self) -> Option<String> {
        if let Some(token) = &self.auth_token {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
        if let Some(var) = &self.auth_token_env
            && let Ok(value) = std::env::var(var)
        {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
        None
    }

    pub fn tool_enabled(&self, remote_name: &str) -> bool {
        self.enabled_tools.is_empty() || self.enabled_tools.iter().any(|name| name == remote_name)
    }
}

/// Parses the conventional `.mcp.json` / `.cursor/mcp.json` shape, keeping
/// only URL-based (streamable HTTP) servers. Command/stdio servers are
/// skipped: this extension only speaks HTTP.
pub fn parse_mcp_servers_json(raw: &str) -> anyhow::Result<Vec<McpServerConfig>> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    let servers = value
        .get("mcpServers")
        .or_else(|| value.get("servers"))
        .and_then(|servers| servers.as_object())
        .cloned()
        .unwrap_or_default();

    let mut configs = Vec::new();
    for (name, entry) in servers {
        let Some(url) = entry.get("url").and_then(|url| url.as_str()) else {
            continue;
        };
        let mut config = McpServerConfig::new(name, url);
        if let Some(headers) = entry.get("headers").and_then(|headers| headers.as_object()) {
            for (key, header_value) in headers {
                if let Some(header_value) = header_value.as_str() {
                    if key.eq_ignore_ascii_case("authorization") {
                        config.auth_token = Some(
                            header_value
                                .trim()
                                .trim_start_matches("Bearer ")
                                .to_string(),
                        );
                    } else {
                        config.headers.insert(key.clone(), header_value.to_string());
                    }
                }
            }
        }
        configs.push(config);
    }
    configs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(configs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_url_servers_and_skips_stdio() {
        let raw = r#"{
            "mcpServers": {
                "vex": {
                    "url": "https://vex.sc/api/v1/mcp",
                    "headers": { "Authorization": "Bearer tok-123", "X-Extra": "1" }
                },
                "local": { "command": "vex", "args": ["mcp", "stdio"] }
            }
        }"#;

        let configs = parse_mcp_servers_json(raw).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "vex");
        assert_eq!(configs[0].url, "https://vex.sc/api/v1/mcp");
        assert_eq!(configs[0].auth_token.as_deref(), Some("tok-123"));
        assert_eq!(
            configs[0].headers.get("X-Extra").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn resolves_token_from_env() {
        let var = "RODER_EXT_MCP_TEST_TOKEN";
        unsafe { std::env::set_var(var, "env-tok") };
        let config = McpServerConfig::new("vex", "http://localhost/mcp").with_auth_token_env(var);
        assert_eq!(config.resolve_auth_token().as_deref(), Some("env-tok"));
        unsafe { std::env::remove_var(var) };
    }

    #[test]
    fn enabled_tools_filter() {
        let mut config = McpServerConfig::new("vex", "http://localhost/mcp");
        assert!(config.tool_enabled("anything"));
        config.enabled_tools = vec!["list_hosted_apps".to_string()];
        assert!(config.tool_enabled("list_hosted_apps"));
        assert!(!config.tool_enabled("create_hosted_app"));
    }

    #[test]
    fn tool_call_auth_mode_defaults_to_configured_credential() {
        let config: McpServerConfig = serde_json::from_value(serde_json::json!({
            "name": "vex",
            "url": "http://localhost/mcp"
        }))
        .unwrap();

        assert_eq!(
            config.tool_call_auth_mode,
            McpToolCallAuthMode::ConfiguredCredential
        );
    }

    #[test]
    fn deserializes_thread_scoped_tool_call_auth_mode() {
        let config: McpServerConfig = serde_json::from_value(serde_json::json!({
            "name": "vex",
            "url": "http://localhost/mcp",
            "tool_call_auth_mode": "thread_scoped_required"
        }))
        .unwrap();

        assert_eq!(
            config.tool_call_auth_mode,
            McpToolCallAuthMode::ThreadScopedRequired
        );
    }
}
