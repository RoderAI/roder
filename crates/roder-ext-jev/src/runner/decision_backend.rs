//! Runtime selection of the browser's decision service.
use super::env_value;
use crate::decide::route::JevHost;
use crate::{
    DecisionsBrowserProfile, JevDecisionClient, JevTypeSafeDecisionClient, OpenAiDecisionsClient,
};
use anyhow::{Context, bail};
use std::sync::Arc;

pub(crate) fn selected() -> String {
    env_value("JEV_DECISION_PROVIDER").unwrap_or_else(|| "jev".into())
}

pub(super) fn resolve() -> anyhow::Result<Arc<dyn JevDecisionClient>> {
    resolve_with(
        &selected(),
        provider_key,
        env_value("JEV_MODEL"),
        env_value("JEV_DECISION_URL"),
    )
}

fn provider_key(provider: &str) -> Option<String> {
    match provider {
        "openai" => {
            env_value("OPENAI_API_KEY").or_else(|| roder_config::provider_api_key("openai"))
        }
        _ => JevHost::parse(provider).and_then(JevHost::key),
    }
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
    url: Option<String>,
) -> anyhow::Result<Arc<dyn JevDecisionClient>> {
    if let Some(host) = JevHost::parse(provider) {
        // TypeSafe image input is a "danger zone" feature: opt-in only, and
        // only on the direct route.
        let images = match env_value("JEV_IMAGES").as_deref() {
            None | Some("0") => false,
            Some("1") if host.takes_images() => true,
            Some("1") => bail!(
                "JEV_IMAGES=1 is not supported with JEV_DECISION_PROVIDER={}",
                host.name()
            ),
            _ => bail!("JEV_IMAGES must be 0 or 1"),
        };
        let profile = prompt_profile(env_value("JEV_PROMPT_PROFILE").as_deref())?;
        let api_key = key(host.name()).with_context(|| {
            format!(
                "{} is required for the {} decision provider; {OTHER_BROWSERS}",
                host.key_name(),
                host.name()
            )
        })?;
        return Ok(Arc::new(
            JevTypeSafeDecisionClient::at(
                &host.endpoint(url),
                api_key,
                model.unwrap_or_else(|| "jev-latest".into()),
            )
            .with_images(images)
            .with_profile(profile),
        ));
    }
    match provider {
        "openai" => {
            let api_key = key("openai").with_context(|| {
                format!(
                    "OPENAI_API_KEY or a configured OpenAI API key is required for Decisions; \
                     Codex sign-in is not supported; {OTHER_BROWSERS}"
                )
            })?;
            let client = OpenAiDecisionsClient::new(api_key).with_browser_profile(
                decisions_profile(env_value("JEV_DECISIONS_PROFILE").as_deref())?,
            );
            let client = match env_value("JEV_DECISIONS_TEXT_ONLY").as_deref() {
                None | Some("0") => client,
                Some("1") => client.text_only(),
                _ => bail!("JEV_DECISIONS_TEXT_ONLY must be 0 or 1"),
            };
            Ok(Arc::new(client))
        }
        _ => bail!("JEV_DECISION_PROVIDER must be jev, openrouter or openai"),
    }
}

fn decisions_profile(raw: Option<&str>) -> anyhow::Result<DecisionsBrowserProfile> {
    match raw {
        None | Some("vision") => Ok(DecisionsBrowserProfile::Visual),
        Some("dom") => Ok(DecisionsBrowserProfile::SemanticDom),
        Some("activation") => Ok(DecisionsBrowserProfile::Audited),
        _ => bail!("JEV_DECISIONS_PROFILE must be vision, dom or activation"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_profiles_are_explicit_and_unknown_values_fail() {
        assert_eq!(
            decisions_profile(None).unwrap(),
            DecisionsBrowserProfile::Visual
        );
        assert_eq!(
            decisions_profile(Some("vision")).unwrap(),
            DecisionsBrowserProfile::Visual
        );
        assert_eq!(
            decisions_profile(Some("dom")).unwrap(),
            DecisionsBrowserProfile::SemanticDom
        );
        assert_eq!(
            decisions_profile(Some("activation")).unwrap(),
            DecisionsBrowserProfile::Audited
        );
        assert!(decisions_profile(Some("typo")).is_err());
    }

    #[test]
    fn openai_requires_only_its_own_key() {
        assert!(
            resolve_with(
                "openai",
                |provider| {
                    assert_eq!(provider, "openai");
                    Some("test-key".into())
                },
                None,
                None
            )
            .is_ok()
        );
        let error = resolve_with("openai", |_| None, None, None).err().unwrap();
        assert!(error.to_string().contains("OPENAI_API_KEY"));
    }

    #[test]
    fn missing_key_errors_name_other_browser_families_without_promising_them() {
        for (provider, key_name) in [
            ("jev", "JEV_API_KEY"),
            ("openrouter", "OPENROUTER_API_KEY"),
            ("openai", "OPENAI_API_KEY"),
        ] {
            let error = resolve_with(provider, |_| None, None, None)
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

    /// OpenRouter takes the same System One body as TypeSafe, at its own
    /// URL and with its own bearer key, never with image input.
    #[tokio::test]
    async fn openrouter_sends_the_system_one_body_with_its_key_and_no_images() {
        use crate::http::tests::{MockServer, Reply};
        use serde_json::json;
        assert_eq!(
            JevHost::OpenRouter.default_endpoint(),
            "https://openrouter.ai/api/v1/systemone"
        );
        assert!(!JevHost::OpenRouter.takes_images());
        let server = MockServer::start(vec![Reply::ok(
            r#"{"answers":{"operation":{"choice":"DONE","confidence":1.0,"probabilities":{"DONE":1.0,"BLOCKED":0.0}}},"model":"typesafe/jev-1.14-20261014","usage":{"input_tokens":328,"output_tokens":0,"cost":0.000013776}}"#,
        )])
        .await;
        let client = resolve_with(
            "openrouter",
            |provider| {
                assert_eq!(provider, "openrouter");
                Some("sk-or-test".into())
            },
            None,
            Some(server.url.clone()),
        )
        .unwrap();
        assert!(!client.uses_images());
        let page = json!({"url":"https://a.test/","title":"A","text":"Thanks","actions":[],
            "_screenshot":"data:image/png;base64,AAAA"});
        let decision = client.choose(&page, "Finish", &[]).await.unwrap();
        assert_eq!(decision.operation, "DONE");
        assert_eq!(
            decision.model.as_deref(),
            Some("typesafe/jev-1.14-20261014")
        );
        assert_eq!(decision.usage["cost"], json!(0.000013776));

        let head = &server.heads()[0];
        assert!(head.starts_with("POST /v1/systemone "), "{head}");
        assert!(!head.to_ascii_lowercase().contains("images"), "{head}");
        let (authorization, body) = &server.requests()[0];
        assert_eq!(authorization, "Bearer sk-or-test");
        assert_eq!(body["model"], "jev-latest");
        assert!(body["state"].is_object() && body["questions"]["operation"].is_object());
        assert!(body.get("assets").is_none());
        assert!(!body.to_string().contains("data:image"));
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
                None,
                None
            )
            .is_ok()
        );
        assert!(
            resolve_with(
                "unknown",
                |_| panic!("must reject before resolving secrets"),
                None,
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
    match JevHost::parse(provider) {
        Some(host) => {
            if host == JevHost::OpenRouter {
                caps.push(CapabilityRequest::new("network.openrouter.ai"));
            }
            caps.push(CapabilityRequest::new(format!(
                "secret.read.{}",
                host.key_name()
            )));
        }
        None if provider == "openai" => {
            caps.push(CapabilityRequest::new("network.api.openai.com"));
            caps.push(CapabilityRequest::new("secret.read.OPENAI_API_KEY"));
        }
        // An unknown provider fails when the tool resolves it.
        None => caps.push(CapabilityRequest::new("secret.read.JEV_API_KEY")),
    }
    caps
}

/// In-memory cache invalidation only; no credential value enters model keys or logs.
pub(super) fn credential_revision() -> u64 {
    credential_fingerprint(provider_key(&selected()).as_deref())
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
        let openrouter = capabilities("openrouter")
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>();
        assert!(openrouter.iter().any(|c| c == "network.openrouter.ai"));
        assert!(
            openrouter
                .iter()
                .any(|c| c == "secret.read.OPENROUTER_API_KEY")
        );
        assert!(!openrouter.iter().any(|c| c.contains("JEV_API_KEY")));
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

/// Keep the measured candidate opt-in while broad validation continues.
fn prompt_profile(raw: Option<&str>) -> anyhow::Result<crate::jev_prompt::Profile> {
    use crate::jev_prompt::Profile;
    match raw {
        None | Some("workflow") => Ok(Profile::Workflow),
        Some("dom") => Ok(Profile::Dom),
        Some("dom_targets") => Ok(Profile::DomTargets),
        Some("activation") => Ok(Profile::Activation),
        _ => bail!("JEV_PROMPT_PROFILE must be workflow, dom, dom_targets or activation"),
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;
    #[test]
    fn dom_evidence_is_an_explicit_profile_and_unknown_profiles_fail() {
        use crate::jev_prompt::Profile;
        assert_eq!(prompt_profile(None).unwrap(), Profile::Workflow);
        assert_eq!(prompt_profile(Some("workflow")).unwrap(), Profile::Workflow);
        assert_eq!(prompt_profile(Some("dom")).unwrap(), Profile::Dom);
        assert_eq!(
            prompt_profile(Some("dom_targets")).unwrap(),
            Profile::DomTargets
        );
        assert_eq!(
            prompt_profile(Some("activation")).unwrap(),
            Profile::Activation
        );
        assert!(prompt_profile(Some("typo")).is_err());
    }
}
