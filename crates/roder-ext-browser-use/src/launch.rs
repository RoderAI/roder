//! Settings for the browser-use provider and the command that launches the
//! browser-use MCP server.
//!
//! The server runs as `uvx --from <package> browser-use --mcp`. Its
//! environment is built from an allowlist of the parent's variables (what
//! `uvx`, Python and the browser need to run) plus the browser-use settings
//! and any LLM keys Roder already holds, so no other Roder secret reaches the
//! child and the keys reach nothing but the child.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use roder_ext_mcp::McpStdioServerConfig;

/// The browser-use release Roder launches unless configured otherwise. The
/// tool table in `catalog.rs` mirrors this release's `tools/list`; bump both
/// together and re-run the ignored live test.
pub const DEFAULT_PACKAGE: &str = "browser-use[cli]==0.13.10";

/// Name used for the server in errors and tool results.
pub const SERVER_NAME: &str = "browser-use";

/// How long the first start may take: `uvx` may download browser-use and its
/// dependencies before the server answers `initialize`.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(300);

/// Resolved settings for the browser-use provider.
#[derive(Clone)]
pub struct BrowserUseConfig {
    /// Run the browser without a window. Off by default so the user sees
    /// what the agent does.
    pub headless: bool,
    /// `uvx --from` requirement, e.g. `browser-use[cli]==0.13.10`.
    pub package: String,
    /// Explicit `uvx` path; otherwise `uvx` is looked up on `PATH`.
    pub uvx: Option<PathBuf>,
    /// Passed to the server as `OPENAI_API_KEY` for its LLM-backed tools.
    pub openai_api_key: Option<String>,
    /// Passed to the server as `ANTHROPIC_API_KEY` for its LLM-backed tools.
    pub anthropic_api_key: Option<String>,
    /// Operator ceiling for every upstream browser session, including the agent.
    pub allowed_domains: Vec<String>,
}

impl std::fmt::Debug for BrowserUseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let present = |key: &Option<String>| if key.is_some() { "[set]" } else { "[unset]" };
        f.debug_struct("BrowserUseConfig")
            .field("headless", &self.headless)
            .field("package", &self.package)
            .field("uvx", &self.uvx)
            .field("allowed_domains", &self.allowed_domains)
            .field("openai_api_key", &present(&self.openai_api_key))
            .field("anthropic_api_key", &present(&self.anthropic_api_key))
            .finish()
    }
}

impl Default for BrowserUseConfig {
    fn default() -> Self {
        Self {
            headless: false,
            package: DEFAULT_PACKAGE.to_string(),
            uvx: None,
            openai_api_key: None,
            anthropic_api_key: None,
            allowed_domains: Vec::new(),
        }
    }
}

impl BrowserUseConfig {
    /// Settings from the process environment: the provider's LLM keys from
    /// `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` and headless mode from
    /// `RODER_BROWSER_USE_HEADLESS`.
    pub fn from_env() -> Self {
        Self {
            headless: std::env::var("RODER_BROWSER_USE_HEADLESS")
                .ok()
                .is_some_and(|value| matches!(value.trim(), "1" | "true" | "yes" | "on")),
            openai_api_key: env_nonempty("OPENAI_API_KEY"),
            anthropic_api_key: env_nonempty("ANTHROPIC_API_KEY"),
            allowed_domains: env_nonempty("RODER_BROWSER_USE_ALLOWED_DOMAINS")
                .map(|list| {
                    list.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            ..Self::default()
        }
    }

    /// Whether the server's LLM-backed tools have a key to use.
    pub fn has_llm_key(&self) -> bool {
        // The pinned MCP tools instantiate ChatOpenAI; an Anthropic key alone
        // cannot run them.
        nonempty(&self.openai_api_key).is_some()
    }

    /// The keys that must never appear in anything Roder reports.
    pub fn secrets(&self) -> Vec<String> {
        [&self.openai_api_key, &self.anthropic_api_key]
            .into_iter()
            .filter_map(|key| nonempty(key).map(str::to_string))
            .collect()
    }
}

/// Finds `uvx`, or explains how to install it.
pub fn resolve_uvx(config: &BrowserUseConfig) -> anyhow::Result<PathBuf> {
    if let Some(path) = &config.uvx {
        if path.is_file() {
            return Ok(path.clone());
        }
        anyhow::bail!(
            "browser-use: the configured uvx path {} does not exist. Fix [browser_use] uvx in \
             Roder config or remove it to look uvx up on PATH.",
            path.display()
        );
    }
    which::which("uvx").map_err(|_| {
        anyhow::anyhow!(
            "browser-use: `uvx` was not found on PATH. The browser-use provider launches its MCP \
             server with uvx, which ships with uv. Install uv (macOS: `brew install uv`; any OS: \
             `curl -LsSf https://astral.sh/uv/install.sh | sh`, see \
             https://docs.astral.sh/uv/getting-started/installation/), or set [browser_use] uvx \
             to its full path."
        )
    })
}

/// Parent variables the server may inherit: what `uvx`, Python, TLS, proxies
/// and a visible browser window need. Nothing else from Roder's environment
/// is passed through.
const INHERITED_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "TEMP",
    "TMP",
    "LANG",
    "TERM",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "DBUS_SESSION_BUS_ADDRESS",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "SYSTEMROOT",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "BROWSER_USE_LOGGING_LEVEL",
];

/// Parent variable prefixes the server may inherit (locale, XDG dirs, uv's
/// own settings such as `UV_CACHE_DIR` and index configuration).
const INHERITED_PREFIXES: &[&str] = &["LC_", "XDG_", "UV_"];

fn inherited(name: &str) -> bool {
    INHERITED_VARS.contains(&name) || INHERITED_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// The server's complete environment, built from `parent` (normally
/// `std::env::vars()`).
pub fn server_env(
    config: &BrowserUseConfig,
    parent: impl IntoIterator<Item = (String, String)>,
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = parent
        .into_iter()
        .filter(|(name, _)| inherited(name))
        .collect();
    env.insert(
        "BROWSER_USE_HEADLESS".into(),
        if config.headless { "true" } else { "false" }.into(),
    );
    // browser-use sends anonymous usage telemetry by default; a coding agent
    // driving it on the user's behalf should not opt them in.
    env.insert("ANONYMIZED_TELEMETRY".into(), "false".into());
    if !config.allowed_domains.is_empty() {
        env.insert(
            "BROWSER_USE_ALLOWED_DOMAINS".into(),
            config.allowed_domains.join(","),
        );
    }
    if let Some(key) = nonempty(&config.openai_api_key) {
        env.insert("OPENAI_API_KEY".into(), key.to_string());
    }
    if let Some(key) = nonempty(&config.anthropic_api_key) {
        env.insert("ANTHROPIC_API_KEY".into(), key.to_string());
    }
    // Never relax the browser's security model on the user's behalf.
    env.remove("BROWSER_USE_DISABLE_SECURITY");
    env
}

/// The launch command for the browser-use MCP server.
pub fn server_command(
    config: &BrowserUseConfig,
    uvx: PathBuf,
    parent_env: impl IntoIterator<Item = (String, String)>,
) -> McpStdioServerConfig {
    McpStdioServerConfig {
        name: SERVER_NAME.to_string(),
        program: uvx,
        args: vec![
            "--from".into(),
            config.package.clone(),
            "browser-use".into(),
            "--mcp".into(),
        ],
        env: server_env(config, parent_env),
        redact: config.secrets(),
        startup_timeout: STARTUP_TIMEOUT,
    }
}

fn nonempty(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parent() -> Vec<(String, String)> {
        [
            ("PATH", "/usr/bin"),
            ("HOME", "/home/me"),
            ("LC_ALL", "C"),
            ("UV_CACHE_DIR", "/cache"),
            ("GITHUB_TOKEN", "ghp-should-not-leak"),
            ("JEV_API_KEY", "jev-should-not-leak"),
            ("OPENAI_API_KEY", "sk-parent-env"),
            ("BROWSER_USE_DISABLE_SECURITY", "true"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn keyed() -> BrowserUseConfig {
        BrowserUseConfig {
            openai_api_key: Some("sk-openai-configured".into()),
            anthropic_api_key: Some("sk-ant-configured".into()),
            ..BrowserUseConfig::default()
        }
    }

    #[test]
    fn command_runs_the_pinned_package_through_uvx() {
        let command = server_command(&BrowserUseConfig::default(), "/opt/uvx".into(), parent());
        assert_eq!(command.program, PathBuf::from("/opt/uvx"));
        assert_eq!(
            command.args,
            ["--from", DEFAULT_PACKAGE, "browser-use", "--mcp"]
        );
        assert_eq!(command.name, "browser-use");
    }

    #[test]
    fn a_configured_package_replaces_the_pin() {
        let config = BrowserUseConfig {
            package: "browser-use[cli]==0.14.0".into(),
            ..BrowserUseConfig::default()
        };
        let command = server_command(&config, "/opt/uvx".into(), parent());
        assert_eq!(command.args[1], "browser-use[cli]==0.14.0");
    }

    #[test]
    fn env_is_an_allowlist_plus_browser_use_settings() {
        let env = server_env(&BrowserUseConfig::default(), parent());
        assert_eq!(env["PATH"], "/usr/bin");
        assert_eq!(env["LC_ALL"], "C");
        assert_eq!(env["UV_CACHE_DIR"], "/cache");
        assert_eq!(env["BROWSER_USE_HEADLESS"], "false");
        assert_eq!(env["ANONYMIZED_TELEMETRY"], "false");
        assert!(!env.contains_key("GITHUB_TOKEN"));
        assert!(!env.contains_key("JEV_API_KEY"));
        assert!(!env.contains_key("BROWSER_USE_DISABLE_SECURITY"));
        // A key only reaches the child when Roder resolved it for OpenAI.
        assert!(!env.contains_key("OPENAI_API_KEY"));
        assert!(!env.contains_key("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn headless_is_opt_in() {
        let config = BrowserUseConfig {
            headless: true,
            ..BrowserUseConfig::default()
        };
        assert_eq!(
            server_env(&config, parent())["BROWSER_USE_HEADLESS"],
            "true"
        );
    }

    #[test]
    fn configured_keys_are_passed_to_the_child_and_marked_for_redaction() {
        let command = server_command(&keyed(), "/opt/uvx".into(), parent());
        assert_eq!(command.env["OPENAI_API_KEY"], "sk-openai-configured");
        assert_eq!(command.env["ANTHROPIC_API_KEY"], "sk-ant-configured");
        assert_eq!(
            command.redact,
            ["sk-openai-configured", "sk-ant-configured"]
        );
        let debug = format!("{command:?} {:?}", keyed());
        assert!(!debug.contains("sk-openai-configured"), "{debug}");
        assert!(!debug.contains("sk-ant-configured"), "{debug}");
    }

    #[test]
    fn blank_keys_count_as_missing() {
        let config = BrowserUseConfig {
            openai_api_key: Some("  ".into()),
            ..BrowserUseConfig::default()
        };
        assert!(!config.has_llm_key());
        assert!(config.secrets().is_empty());
        assert!(!server_env(&config, parent()).contains_key("OPENAI_API_KEY"));
        assert!(keyed().has_llm_key());
        assert!(
            !BrowserUseConfig {
                anthropic_api_key: Some("test-anthropic".into()),
                ..Default::default()
            }
            .has_llm_key()
        );
    }

    #[test]
    fn a_missing_configured_uvx_is_explained() {
        let config = BrowserUseConfig {
            uvx: Some("/definitely/not/uvx".into()),
            ..BrowserUseConfig::default()
        };
        let error = resolve_uvx(&config).unwrap_err().to_string();
        assert!(error.contains("/definitely/not/uvx"), "{error}");
    }
}
