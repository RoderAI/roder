//! Where hosted Jev decisions are sent. TypeSafe serves Jev directly and
//! OpenRouter resells it at the same System One API: the request body is the
//! same, only the URL and the bearer key differ. OpenRouter is sent no image
//! input, a TypeSafe opt-in feature it does not carry.

use crate::runner::env_value;

pub(crate) const TYPESAFE_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
pub(crate) const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/v1/systemone";

/// A host that serves Jev's System One API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JevHost {
    /// `JEV_DECISION_PROVIDER=jev` (the default): TypeSafe directly.
    TypeSafe,
    /// `JEV_DECISION_PROVIDER=openrouter`: Jev through OpenRouter.
    OpenRouter,
}

impl JevHost {
    /// The `JEV_DECISION_PROVIDER` value that names a hosted Jev route.
    pub(crate) fn parse(provider: &str) -> Option<Self> {
        match provider {
            "jev" => Some(Self::TypeSafe),
            "openrouter" => Some(Self::OpenRouter),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::TypeSafe => "jev",
            Self::OpenRouter => "openrouter",
        }
    }

    /// The environment variable that holds this host's key.
    pub(crate) fn key_name(self) -> &'static str {
        match self {
            Self::TypeSafe => "JEV_API_KEY",
            Self::OpenRouter => "OPENROUTER_API_KEY",
        }
    }

    /// Whether `JEV_IMAGES=1` may send screenshots on this route.
    pub(crate) fn takes_images(self) -> bool {
        self == Self::TypeSafe
    }

    /// This host's System One URL, or `url` (`JEV_DECISION_URL`) when set.
    pub(crate) fn endpoint(self, url: Option<String>) -> String {
        url.unwrap_or_else(|| self.default_endpoint().to_string())
    }

    pub(crate) fn default_endpoint(self) -> &'static str {
        match self {
            Self::TypeSafe => TYPESAFE_ENDPOINT,
            Self::OpenRouter => OPENROUTER_ENDPOINT,
        }
    }

    /// The key from the environment, else Roder's provider config.
    pub(crate) fn key(self) -> Option<String> {
        match self {
            Self::TypeSafe => {
                env_value("JEV_API_KEY").or_else(|| roder_config::provider_api_key("jev"))
            }
            Self::OpenRouter => env_value("OPENROUTER_API_KEY")
                .or_else(|| roder_config::provider_api_key("openrouter")),
        }
    }
}

/// The hosted route an eval tier sends decisions on: `JEV_DECISION_PROVIDER`
/// picks the host (`jev` by default) and `JEV_DECISION_URL` may override its
/// URL. The direct route also takes `TYPESAFE_API_KEY`.
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct HostedRoute {
    pub(crate) host: JevHost,
    pub(crate) url: String,
    pub(crate) key: String,
}

/// Never prints the key.
#[cfg(test)]
impl std::fmt::Debug for HostedRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostedRoute")
            .field("host", &self.host)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
impl HostedRoute {
    /// `None` when the selected host has no key, or the provider is not a
    /// hosted Jev route.
    pub(crate) fn from_env() -> Option<Self> {
        let provider = env_value("JEV_DECISION_PROVIDER").unwrap_or_else(|| "jev".into());
        let host = JevHost::parse(&provider)?;
        let key = match host {
            JevHost::TypeSafe => env_value("TYPESAFE_API_KEY").or_else(|| host.key()),
            JevHost::OpenRouter => host.key(),
        }?;
        Some(Self {
            host,
            url: host.endpoint(env_value("JEV_DECISION_URL")),
            key,
        })
    }

    /// What to set to run a live tier, for its skip message.
    pub(crate) const NEEDS: &'static str = "TYPESAFE_API_KEY (or JEV_API_KEY), or \
         JEV_DECISION_PROVIDER=openrouter with OPENROUTER_API_KEY";

    pub(crate) fn transport(&self) -> super::TypeSafeHttpTransport {
        super::TypeSafeHttpTransport::new(
            &self.url,
            self.key.clone(),
            crate::http::RetryPolicy::default(),
        )
    }

    pub(crate) fn client(&self, model: impl Into<String>) -> super::JevTypeSafeDecisionClient {
        super::JevTypeSafeDecisionClient::at(&self.url, self.key.clone(), model)
    }
}
