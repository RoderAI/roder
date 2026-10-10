//! Runtime selection of the browser's decision service.
use super::{env_value, key_from_env_or_config};
use crate::{JevDecisionClient, JevTypeSafeDecisionClient, OpenAiDecisionsClient};
use anyhow::{Context, bail};
use std::sync::Arc;

pub(crate) fn selected() -> String {
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

/// What to do when Jev cannot start for want of a key. Families are named by
/// tool-name prefix only: this crate cannot see which of them are advertised
/// or working.
const OTHER_BROWSERS: &str = "otherwise use another browser tool family if one is available \
     (chrome_*, browser_use_*, webwright.*)";

fn resolve_with(
    provider: &str,
    key: impl Fn(&str) -> Option<String>,
    model: Option<String>,
) -> anyhow::Result<Arc<dyn JevDecisionClient>> {
    match provider {
        "jev" => Ok(Arc::new(JevTypeSafeDecisionClient::new(
            key("jev").with_context(|| {
                format!("JEV_API_KEY is required for the Jev decision provider; {OTHER_BROWSERS}")
            })?,
            model.unwrap_or_else(|| "jev-latest".into()),
        ))),
        "openai" => {
            let api_key = key("openai").with_context(|| {
                format!(
                    "OPENAI_API_KEY or a configured OpenAI API key is required for Decisions; \
                     Codex sign-in is not supported; {OTHER_BROWSERS}"
                )
            })?;
            let client = OpenAiDecisionsClient::new(api_key);
            let client = match env_value("JEV_DECISIONS_TEXT_ONLY").as_deref() {
                None | Some("0") => client,
                Some("1") => client.text_only(),
                _ => bail!("JEV_DECISIONS_TEXT_ONLY must be 0 or 1"),
            };
            Ok(Arc::new(client))
        }
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
    fn missing_key_errors_name_other_browser_families_without_promising_them() {
        for (provider, key_name) in [("jev", "JEV_API_KEY"), ("openai", "OPENAI_API_KEY")] {
            let error = resolve_with(provider, |_| None, None)
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains(key_name), "{error}");
            for prefix in ["chrome_*", "browser_use_*", "webwright.*"] {
                assert!(error.contains(prefix), "{provider} {prefix}: {error}");
            }
            assert!(error.contains("if one is available"), "{error}");
        }
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

/// Provider-specific requirements are requested only by the selected backend.
pub(crate) fn capabilities(provider: &str) -> Vec<roder_api::capabilities::CapabilityRequest> {
    use roder_api::capabilities::CapabilityRequest;
    let mut caps = vec![CapabilityRequest::new("network.web")];
    if provider == "openai" {
        caps.push(CapabilityRequest::new("network.api.openai.com"));
        caps.push(CapabilityRequest::new("secret.read.OPENAI_API_KEY"));
    } else {
        caps.push(CapabilityRequest::new("secret.read.JEV_API_KEY"));
    }
    caps
}

/// In-memory cache invalidation only; no credential value enters model keys or logs.
pub(super) fn credential_revision() -> u64 {
    let key = match selected().as_str() {
        "openai" => {
            env_value("OPENAI_API_KEY").or_else(|| roder_config::provider_api_key("openai"))
        }
        "jev" => key_from_env_or_config(),
        _ => None,
    };
    credential_fingerprint(key.as_deref())
}
fn credential_fingerprint(key: Option<&str>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hash);
    hash.finish()
}
#[cfg(test)]
mod capability_tests {
    use super::*;
    #[test]
    fn only_selected_backend_requests_its_secret_and_network() {
        let jev = capabilities("jev")
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>();
        let openai = capabilities("openai")
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>();
        assert!(
            !jev.iter()
                .any(|c| c.contains("OPENAI") || c.contains("openai"))
        );
        assert!(openai.iter().any(|c| c == "network.api.openai.com"));
        assert!(!openai.iter().any(|c| c == "secret.read.JEV_API_KEY"));
    }
    #[test]
    fn rotating_or_removing_credentials_changes_cache_revision() {
        assert_ne!(
            credential_fingerprint(Some("first")),
            credential_fingerprint(Some("second"))
        );
        assert_ne!(
            credential_fingerprint(Some("first")),
            credential_fingerprint(None)
        );
    }
}
