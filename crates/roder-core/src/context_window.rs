use roder_api::catalog::{effective_model_context_window, lookup_model_for_provider};

use crate::{Runtime, RuntimeConfig};

impl Runtime {
    pub async fn set_model_context_window(
        &self,
        window: Option<u32>,
    ) -> anyhow::Result<RuntimeConfig> {
        anyhow::ensure!(window != Some(0), "model_context_window must be positive");
        let mut cfg = self.config.write().await;
        cfg.model_context_window = window;
        Ok(cfg.clone())
    }
}

/// Return context and compaction budgets using the same policy for native and
/// delegated GPT runtimes. Explicit compaction settings remain authoritative,
/// but cannot put compaction beyond the effective context window.
pub fn model_context_budgets(
    cfg: &RuntimeConfig,
    provider: &str,
    model: &str,
) -> Option<(u32, u32)> {
    if !matches!(provider, "openai" | "codex") || !model.starts_with("gpt-") {
        return None;
    }
    let entry = lookup_model_for_provider(provider, model)?;
    let window = effective_model_context_window(provider, model, cfg.model_context_window)?;
    let default_threshold = window.saturating_mul(9) / 10;
    let threshold = cfg
        .auto_compact_token_limit
        .or_else(|| cfg.model_auto_compact_token_limits.get(model).copied())
        .or_else(|| {
            cfg.model_profiles.get(model)
            .and_then(|profile| profile.auto_compact_token_limit)
            // Resolved profiles include inherited catalog thresholds. They
            // are defaults, not explicit overrides of the context setting.
            .filter(|threshold| *threshold != entry.auto_compact_token_limit)
        })
        .unwrap_or(if window == entry.context_window {
            entry.auto_compact_token_limit
        } else {
            default_threshold.max(1)
        })
        .min(window);
    Some((window, threshold))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_change_budget_clamp_to_max_and_preserve_explicit_compaction() {
        let mut cfg = RuntimeConfig {
            model_context_window: Some(200_000),
            ..Default::default()
        };
        assert_eq!(
            model_context_budgets(&cfg, "codex", "gpt-6.2-sol"),
            Some((200_000, 180_000))
        );
        cfg.auto_compact_token_limit = Some(150_000);
        assert_eq!(
            model_context_budgets(&cfg, "openai", "gpt-6.2-sol"),
            Some((200_000, 150_000))
        );
        cfg.auto_compact_token_limit = Some(900_000);
        assert_eq!(
            model_context_budgets(&cfg, "codex", "gpt-6.2-sol"),
            Some((200_000, 200_000))
        );
        cfg.model_context_window = Some(u32::MAX);
        assert_eq!(
            model_context_budgets(&cfg, "codex", "gpt-6.2-sol")
                .unwrap()
                .0,
            872_000
        );
        assert_eq!(
            model_context_budgets(&cfg, "anthropic", "gpt-6.2-sol"),
            None
        );
        assert_eq!(model_context_budgets(&cfg, "openai", "o3"), None);
        cfg.model_context_window = None;
        cfg.auto_compact_token_limit = None;
        assert_eq!(
            model_context_budgets(&cfg, "codex", "gpt-6.2-astra"),
            Some((500_000, 450_000))
        );
    }

    #[test]
    fn inherited_profile_limits_scale_but_explicit_profile_limits_do_not() {
        let mut cfg = RuntimeConfig {
            model_context_window: Some(200_000),
            model_profiles: crate::model_profiles::resolve_model_profiles(&Default::default())
                .unwrap(),
            ..Default::default()
        };
        assert_eq!(
            model_context_budgets(&cfg, "codex", "gpt-6.2-sol"),
            Some((200_000, 180_000))
        );
        cfg.model_auto_compact_token_limits
            .insert("gpt-6.2-sol".into(), 450_000);
        assert_eq!(
            model_context_budgets(&cfg, "codex", "gpt-6.2-sol"),
            Some((200_000, 200_000))
        );
        cfg.model_auto_compact_token_limits
            .insert("gpt-6.2-sol".into(), 75_000);
        assert_eq!(
            model_context_budgets(&cfg, "codex", "gpt-6.2-sol"),
            Some((200_000, 75_000))
        );
        cfg.auto_compact_token_limit = Some(0);
        assert_eq!(
            model_context_budgets(&cfg, "codex", "gpt-6.2-sol"),
            Some((200_000, 0))
        );
    }

    #[tokio::test]
    async fn live_context_override_can_be_changed_and_reset() {
        let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
        builder.inference_engine(std::sync::Arc::new(
            crate::fake_provider::FakeInferenceEngine,
        ));
        let registry = builder.build().unwrap();
        assert!(
            Runtime::new(
                registry.clone(),
                RuntimeConfig {
                    model_context_window: Some(0),
                    ..Default::default()
                }
            )
            .is_err()
        );
        let runtime = Runtime::new(registry, RuntimeConfig::default()).unwrap();
        runtime
            .set_model_context_window(Some(200_000))
            .await
            .unwrap();
        assert_eq!(
            model_context_budgets(&runtime.status().await, "codex", "gpt-6.2-sol"),
            Some((200_000, 180_000))
        );
        assert!(runtime.set_model_context_window(Some(0)).await.is_err());
        assert_eq!(runtime.status().await.model_context_window, Some(200_000));
        runtime.set_model_context_window(None).await.unwrap();
        assert_eq!(
            model_context_budgets(&runtime.status().await, "codex", "gpt-6.2-sol"),
            Some((500_000, 450_000))
        );
    }
}
