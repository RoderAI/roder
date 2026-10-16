use crate::{Redacted, config::DEFAULT_BASE_URL};

/// An operator-selected account for source creation and subsequent cleanup.
/// Credentials stay in memory; neither destination nor process environment can
/// redirect this profile to a different workspace or endpoint.
#[derive(Clone, Debug)]
pub struct BlaxelWorkspace {
    pub(crate) token: Redacted,
    pub(crate) workspace: String,
    pub(crate) base_url: String,
}

impl BlaxelWorkspace {
    pub fn new(token: Redacted, workspace: impl Into<String>) -> anyhow::Result<Self> {
        Self::with_base_url(token, workspace, DEFAULT_BASE_URL)
    }

    /// Production uses the Blaxel control plane. Loopback is for transport tests.
    pub fn with_base_url(
        token: Redacted,
        workspace: impl Into<String>,
        base_url: &str,
    ) -> anyhow::Result<Self> {
        let workspace = workspace.into();
        anyhow::ensure!(
            !workspace.is_empty()
                && workspace.len() <= 63
                && workspace
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !workspace.starts_with('-')
                && !workspace.ends_with('-'),
            "invalid provider workspace"
        );
        anyhow::ensure!(
            !token.is_empty() && reqwest::header::HeaderValue::from_str(token.as_str()).is_ok(),
            "invalid workspace credential"
        );
        let url = reqwest::Url::parse(base_url)
            .map_err(|_| anyhow::anyhow!("invalid control plane URL"))?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        anyhow::ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path().trim_end_matches('/') == "/v0"
                && ((url.scheme() == "https"
                    && url.host_str() == Some("api.blaxel.ai")
                    && url.port().is_none())
                    || (url.scheme() == "http" && loopback)),
            "unsupported control plane URL"
        );
        Ok(Self {
            token,
            workspace,
            base_url: base_url.trim_end_matches('/').into(),
        })
    }

    pub(crate) fn validate_endpoint(&self, endpoint: &str) -> anyhow::Result<()> {
        validate_endpoint(&self.base_url, endpoint)
    }

    pub(crate) fn assert_scope(&self, config: &serde_json::Value) -> anyhow::Result<()> {
        for (key, expected) in [
            ("workspace", self.workspace.as_str()),
            ("base_url", self.base_url.as_str()),
        ] {
            if let Some(value) = config.get(key) {
                anyhow::ensure!(
                    value.as_str() == Some(expected),
                    "provider {key} differs from explicit workspace"
                );
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_endpoint(base_url: &str, endpoint: &str) -> anyhow::Result<()> {
    let url =
        reqwest::Url::parse(endpoint).map_err(|_| anyhow::anyhow!("invalid sandbox endpoint"))?;
    let provider = url.scheme() == "https"
        && url.host_str().is_some_and(|host| host.ends_with(".bl.run"))
        && url.port().is_none();
    let local = base_url.starts_with("http://")
        && reqwest::Url::parse(base_url)
            .ok()
            .is_some_and(|control| control.origin() == url.origin());
    anyhow::ensure!(
        (provider || local)
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && matches!(url.path(), "" | "/"),
        "unsupported sandbox endpoint"
    );
    Ok(())
}
