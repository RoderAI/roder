use roder_api::catalog::{
    PROVIDER_XIAOMI_MIMO, PROVIDER_XIAOMI_MIMO_TOKEN_PLAN, models_for_provider,
};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext, ModelDescriptor,
    ProviderAuthType,
};
use roder_ext_openai_chat_completions::{ChatCompletionsRequestConfig, stream_chat_completions};

const DEFAULT_BASE_URL: &str = "https://api.xiaomimimo.com/v1";

#[derive(Debug, Clone, Default)]
pub struct XiaomiMimoConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub token_plan_api_key: Option<String>,
    pub token_plan_base_url: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct XiaomiMimoProviderSpec {
    pub provider_id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub sort_order: i32,
    pub default_base_url: Option<&'static str>,
    pub api_key_env: &'static str,
    pub api_key_aliases: &'static [&'static str],
    pub base_url_env: &'static str,
    pub base_url_aliases: &'static [&'static str],
    pub token_plan: bool,
}

impl XiaomiMimoProviderSpec {
    pub fn pay_as_you_go() -> Self {
        Self {
            provider_id: PROVIDER_XIAOMI_MIMO,
            name: "Xiaomi MiMo",
            description: "Xiaomi MiMo pay-as-you-go OpenAI-compatible Chat Completions provider",
            sort_order: 18,
            default_base_url: Some(DEFAULT_BASE_URL),
            api_key_env: "MIMO_API_KEY",
            api_key_aliases: &["XIAOMI_MIMO_API_KEY", "RODER_XIAOMI_MIMO_API_KEY"],
            base_url_env: "MIMO_BASE_URL",
            base_url_aliases: &["XIAOMI_MIMO_BASE_URL", "RODER_XIAOMI_MIMO_BASE_URL"],
            token_plan: false,
        }
    }

    pub fn token_plan() -> Self {
        Self {
            provider_id: PROVIDER_XIAOMI_MIMO_TOKEN_PLAN,
            name: "Xiaomi MiMo Token Plan",
            description: "Xiaomi MiMo Token Plan subscription Chat Completions provider",
            sort_order: 19,
            default_base_url: None,
            api_key_env: "MIMO_TOKEN_PLAN_API_KEY",
            api_key_aliases: &[
                "XIAOMI_MIMO_TOKEN_PLAN_API_KEY",
                "RODER_XIAOMI_MIMO_TOKEN_PLAN_API_KEY",
            ],
            base_url_env: "MIMO_TOKEN_PLAN_BASE_URL",
            base_url_aliases: &[
                "XIAOMI_MIMO_TOKEN_PLAN_BASE_URL",
                "RODER_XIAOMI_MIMO_TOKEN_PLAN_BASE_URL",
            ],
            token_plan: true,
        }
    }
}

pub struct XiaomiMimoInferenceEngine {
    config: XiaomiMimoConfig,
    spec: XiaomiMimoProviderSpec,
}

impl XiaomiMimoInferenceEngine {
    pub fn new(config: XiaomiMimoConfig, spec: XiaomiMimoProviderSpec) -> Self {
        Self { config, spec }
    }

    pub(crate) fn api_key(&self) -> Option<String> {
        let configured = if self.spec.token_plan {
            self.config.token_plan_api_key.clone()
        } else {
            self.config.api_key.clone()
        };
        nonempty(configured)
            .or_else(|| env_nonempty(self.spec.api_key_env))
            .or_else(|| first_env_nonempty(self.spec.api_key_aliases))
            .or_else(|| config_api_key(self.spec.provider_id))
    }

    pub(crate) fn base_url(&self) -> Option<String> {
        let configured = if self.spec.token_plan {
            self.config.token_plan_base_url.clone()
        } else {
            self.config.base_url.clone()
        };
        nonempty(configured)
            .or_else(|| config_base_url(self.spec.provider_id))
            .or_else(|| env_nonempty(self.spec.base_url_env))
            .or_else(|| first_env_nonempty(self.spec.base_url_aliases))
            .or_else(|| self.spec.default_base_url.map(ToString::to_string))
            .map(|value| value.trim_end_matches('/').to_string())
    }

    pub(crate) fn validate_token_plan(&self, api_key: &str, base_url: &str) -> anyhow::Result<()> {
        validate_token_plan_auth(api_key, base_url)
    }
}

#[async_trait::async_trait]
impl InferenceEngine for XiaomiMimoInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        self.spec.provider_id.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: false,
            structured_output: true,
            image_input: true,
            prompt_cache: true,
            provider_metadata: true,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        let auth_configured = match (self.api_key(), self.base_url()) {
            (Some(api_key), Some(base_url)) if self.spec.token_plan => {
                validate_token_plan_auth(&api_key, &base_url).is_ok()
            }
            (Some(_), Some(_)) => true,
            _ => false,
        };
        InferenceProviderMetadata {
            name: self.spec.name.to_string(),
            description: Some(self.spec.description.to_string()),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some(self.spec.api_key_env.to_string()),
            auth_configured: Some(auth_configured),
            recommended: true,
            sort_order: self.spec.sort_order,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(self.spec.provider_id, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let Some(api_key) = self.api_key() else {
            anyhow::bail!(
                "{} API key is missing; set {} or configure it from the provider menu",
                self.spec.name,
                self.spec.api_key_env
            )
        };
        let Some(base_url) = self.base_url() else {
            anyhow::bail!(
                "{} base URL is missing; set {} from the Token Plan subscription page",
                self.spec.name,
                self.spec.base_url_env
            )
        };
        if self.spec.token_plan {
            self.validate_token_plan(&api_key, &base_url)?;
        }
        stream_chat_completions(
            ChatCompletionsRequestConfig::api_key_header(self.spec.name, base_url, api_key),
            request,
        )
        .await
    }
}

pub(crate) fn validate_token_plan_auth(api_key: &str, base_url: &str) -> anyhow::Result<()> {
    if !api_key.starts_with("tp-") {
        anyhow::bail!("Xiaomi MiMo Token Plan API key must start with `tp-`");
    }
    if !is_allowed_token_plan_base_url(base_url) {
        anyhow::bail!(
            "Xiaomi MiMo Token Plan base URL must be the exclusive token-plan OpenAI-compatible URL from the subscription page"
        );
    }
    Ok(())
}

pub(crate) fn is_allowed_token_plan_base_url(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    matches!(host, "127.0.0.1" | "localhost")
        || host.starts_with("127.")
        || host.ends_with(".localhost")
        || matches!(
            host,
            "token-plan-cn.xiaomimimo.com"
                | "token-plan-sgp.xiaomimimo.com"
                | "token-plan-ams.xiaomimimo.com"
        )
}

fn config_api_key(provider_id: &str) -> Option<String> {
    roder_config::load_config().ok().and_then(|config| {
        config.providers.get(provider_id).and_then(|provider| {
            nonempty(provider.api_key.clone())
                .or_else(|| provider.api_key_env.as_deref().and_then(env_nonempty))
        })
    })
}

fn config_base_url(provider_id: &str) -> Option<String> {
    roder_config::load_config().ok().and_then(|config| {
        config
            .providers
            .get(provider_id)
            .and_then(|provider| nonempty(provider.base_url.clone()))
    })
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .and_then(|value| nonempty(Some(value)))
}

fn first_env_nonempty(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| env_nonempty(key))
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_plan_requires_tp_key_and_official_or_local_base_url() {
        assert!(
            validate_token_plan_auth("tp-secret", "https://token-plan-cn.xiaomimimo.com/v1")
                .is_ok()
        );
        assert!(
            validate_token_plan_auth("sk-secret", "https://token-plan-cn.xiaomimimo.com/v1")
                .is_err()
        );
        assert!(validate_token_plan_auth("tp-secret", "https://api.xiaomimimo.com/v1").is_err());
        assert!(validate_token_plan_auth("tp-secret", "http://127.0.0.1:8080/v1").is_ok());
    }

    #[tokio::test]
    async fn xiaomi_models_are_catalog_models() {
        let engine = XiaomiMimoInferenceEngine::new(
            XiaomiMimoConfig::default(),
            XiaomiMimoProviderSpec::pay_as_you_go(),
        );
        let models = engine
            .list_models(InferenceProviderContext {
                provider_id: PROVIDER_XIAOMI_MIMO,
            })
            .await
            .unwrap();

        assert!(models.iter().any(|model| model.id == "mimo-v2.5-pro"));
        assert!(models.iter().any(|model| model.id == "mimo-v2-flash"));
        assert!(!models.iter().any(|model| model.id == "mimo-v2.5-tts"));
    }
}
