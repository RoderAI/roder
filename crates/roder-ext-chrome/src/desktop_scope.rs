//! Optional operator ceiling for the integrated Desktop browser.
use crate::direct::DirectGuard;

pub(crate) struct DesktopScope(Vec<String>);

impl DesktopScope {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let Some(raw) = std::env::var("RODER_DESKTOP_ALLOWED_ORIGINS").ok() else {
            return Ok(Self(Vec::new()));
        };
        let origins = raw
            .split([',', ' ', '\n', '\t'])
            .filter(|s| !s.is_empty())
            .map(|raw| {
                let url = reqwest::Url::parse(raw)?;
                anyhow::ensure!(
                    matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.path() == "/"
                        && url.query().is_none()
                        && url.fragment().is_none(),
                    "RODER_DESKTOP_ALLOWED_ORIGINS must contain exact http(s) origins"
                );
                Ok(url.origin().ascii_serialization())
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        anyhow::ensure!(
            !origins.is_empty(),
            "RODER_DESKTOP_ALLOWED_ORIGINS must not be empty"
        );
        Ok(Self(origins))
    }
}

impl DirectGuard for DesktopScope {
    fn outside(&self, url: &str) -> Option<String> {
        if self.0.is_empty() || url == "about:blank" {
            return None;
        }
        let origin = reqwest::Url::parse(url)
            .ok()
            .map(|url| url.origin().ascii_serialization());
        (!origin.is_some_and(|origin| self.0.contains(&origin)))
            .then(|| "Desktop page is outside RODER_DESKTOP_ALLOWED_ORIGINS; action refused".into())
    }
}
