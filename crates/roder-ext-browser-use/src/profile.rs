//! Upstream MCP defaults to a shared user profile; override it explicitly.
use std::path::PathBuf;

use roder_ext_mcp::McpStdioServerConfig;
use serde_json::json;

pub(crate) struct OwnedProfile(PathBuf);

impl OwnedProfile {
    pub(crate) fn configure(spec: &mut McpStdioServerConfig) -> anyhow::Result<Self> {
        if let Some(raw) = spec.env.get("BROWSER_USE_ALLOWED_DOMAINS") {
            let domains = raw
                .split(',')
                .map(|domain| {
                    let url = reqwest::Url::parse(&format!("https://{domain}"))?;
                    anyhow::ensure!(
                        url.path() == "/"
                            && url.query().is_none()
                            && url.fragment().is_none()
                            && url.username().is_empty()
                            && url.password().is_none()
                            && url.port().is_none()
                            && url.host_str().is_some()
                            && !domain.contains('*'),
                        "RODER_BROWSER_USE_ALLOWED_DOMAINS accepts exact hostnames only"
                    );
                    Ok(url.host_str().unwrap().to_string())
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            spec.env
                .insert("BROWSER_USE_ALLOWED_DOMAINS".into(), domains.join(","));
        }
        let root = std::env::temp_dir().join(format!("roder-browser-use-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        }
        let profile = Self(root);
        // The pinned server initializes extract_content from config only;
        // its autonomous agent also accepts the environment key. Keep the
        // resolved key in this private, ephemeral config, never a report.
        let llm = spec.env.get("OPENAI_API_KEY").map_or(json!({}), |key| {
            json!({
                "roder": {"id":"roder", "default":true, "model":"gpt-4.1-mini", "api_key":key}
            })
        });
        let config = json!({
            "browser_profile": {"roder": {
                "id":"roder", "default":true,
                "user_data_dir":profile.0.join("browser"),
                "downloads_path":profile.0.join("downloads"),
                "file_system_path":profile.0.join("files"),
                "disable_security":false, "keep_alive":false,
                "enable_default_extensions":false
            }},
            "llm":llm, "agent":{}
        });
        let path = profile.0.join("config.json");
        std::fs::write(&path, serde_json::to_vec(&config)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        spec.env.insert(
            "BROWSER_USE_CONFIG_PATH".into(),
            path.to_string_lossy().into_owned(),
        );
        spec.env.insert(
            "BROWSER_USE_CONFIG_DIR".into(),
            profile.0.to_string_lossy().into_owned(),
        );
        Ok(profile)
    }
}

impl Drop for OwnedProfile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
