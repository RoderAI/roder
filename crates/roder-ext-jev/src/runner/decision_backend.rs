//! Runtime selection of the browser's decision service.
use super::{env_value, key_from_env_or_config};
use crate::{JevDecisionClient, JevTypeSafeDecisionClient, OpenAiDecisionsClient};
use anyhow::{Context, bail};
use std::sync::Arc;

pub(super) fn selected() -> String {
    env_value("JEV_DECISION_PROVIDER").unwrap_or_else(|| "jev".into())
}

pub(super) fn resolve() -> anyhow::Result<Arc<dyn JevDecisionClient>> {
    resolve_with(
        &selected(),
        |name| match name {
            "jev" => key_from_env_or_config(),
            "openai" => {
                env_value("OPENAI_API_KEY").or_else(|| roder_config::provider_api_key("openai"))
            }
            _ => None,
        },
        env_value("JEV_MODEL"),
    )
}

fn resolve_with(
    provider: &str,
    key: impl Fn(&str) -> Option<String>,
    model: Option<String>,
) -> anyhow::Result<Arc<dyn JevDecisionClient>> {
    match provider {
        "jev" => Ok(Arc::new(JevTypeSafeDecisionClient::new(
            key("jev").context("JEV_API_KEY is required for the Jev decision provider")?,
            model.unwrap_or_else(|| "jev-latest".into()),
        ))),
        "openai" => Ok(Arc::new(OpenAiDecisionsClient::new(
            key("openai").context("OPENAI_API_KEY or a configured OpenAI API key is required for Decisions; Codex sign-in is not supported")?,
        ))),
        _ => bail!("JEV_DECISION_PROVIDER must be jev or openai"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_requires_only_its_own_key() {
        assert!(
            resolve_with(
                "openai",
                |provider| {
                    assert_eq!(provider, "openai");
                    Some("test-key".into())
                },
                None
            )
            .is_ok()
        );
        let error = resolve_with("openai", |_| None, None).err().unwrap();
        assert!(error.to_string().contains("OPENAI_API_KEY"));
    }

    #[test]
    fn jev_and_invalid_provider_selection_are_explicit() {
        assert!(
            resolve_with(
                "jev",
                |provider| {
                    assert_eq!(provider, "jev");
                    Some("test-key".into())
                },
                None
            )
            .is_ok()
        );
        assert!(
            resolve_with(
                "unknown",
                |_| panic!("must reject before resolving secrets"),
                None
            )
            .is_err()
        );
    }
}
