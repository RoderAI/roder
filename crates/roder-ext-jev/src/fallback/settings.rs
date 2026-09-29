//! What the operator sets for the fallback: whether it runs, which model
//! drives it, and its own ceilings. A malformed value fails every call, like
//! Jev's other operator settings, rather than running without the limit.

use std::time::Duration;

use anyhow::{Context, bail};

use crate::runner::env_value;
use crate::text_model::Effort;

/// `JEV_FALLBACK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FallbackMode {
    /// When Jev cannot progress, `jev_browse` itself goes on with a model
    /// and the full browser tools, in the same tab, and returns one result.
    Auto,
    /// Jev's result tells the caller the full tools are available on the
    /// same tab, and the caller goes on with them.
    Handover,
    /// Neither: Jev's result as it was.
    Off,
}

impl FallbackMode {
    pub(crate) fn parse(raw: Option<&str>) -> anyhow::Result<Self> {
        Ok(
            match raw.map(|raw| raw.trim().to_ascii_lowercase()).as_deref() {
                None | Some("auto") => Self::Auto,
                Some("handover") => Self::Handover,
                Some("off") => Self::Off,
                Some(other) => bail!("JEV_FALLBACK must be auto, handover or off, not {other:?}"),
            },
        )
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Handover => "handover",
            Self::Off => "off",
        }
    }
}

/// A model named in `JEV_FALLBACK_MODEL`: `provider/model`, or a model id
/// Roder's catalog knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PinnedModel {
    pub(crate) provider: Option<String>,
    pub(crate) model: String,
}

impl PinnedModel {
    fn parse(raw: &str) -> anyhow::Result<Self> {
        let raw = raw.trim();
        let pinned = match raw.split_once('/') {
            Some((provider, model)) if !provider.is_empty() && !model.is_empty() => Self {
                provider: Some(provider.to_string()),
                model: model.to_string(),
            },
            Some(_) => {
                bail!("JEV_FALLBACK_MODEL must be provider/model or a model id, not {raw:?}")
            }
            None => Self {
                provider: None,
                model: raw.to_string(),
            },
        };
        Ok(pinned)
    }
}

/// Defaults: a fallback may take 20 tool calls, 120 s and 400,000 tokens.
pub(crate) const DEFAULT_STEPS: usize = 20;
pub(crate) const DEFAULT_SECONDS: u64 = 120;
pub(crate) const DEFAULT_TOKENS: u64 = 400_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FallbackSettings {
    pub(crate) mode: FallbackMode,
    /// `JEV_FALLBACK_MODEL`; `None` drives the fallback with the model the
    /// session is on.
    pub(crate) model: Option<PinnedModel>,
    /// `JEV_FALLBACK_REASONING`: the effort asked of the fallback model.
    pub(crate) effort: Effort,
    /// `JEV_FALLBACK_MAX_STEPS`: tool calls one fallback may make.
    pub(crate) max_steps: usize,
    /// `JEV_FALLBACK_MAX_SECONDS`: the longest one fallback may run, within
    /// what is left of the host's deadline.
    pub(crate) max_duration: Duration,
    /// `JEV_FALLBACK_MAX_TOKENS`: input and output tokens one fallback may
    /// spend, summed over its model calls.
    pub(crate) max_tokens: u64,
}

impl Default for FallbackSettings {
    fn default() -> Self {
        Self {
            mode: FallbackMode::Auto,
            model: None,
            effort: Effort::Low,
            max_steps: DEFAULT_STEPS,
            max_duration: Duration::from_secs(DEFAULT_SECONDS),
            max_tokens: DEFAULT_TOKENS,
        }
    }
}

impl FallbackSettings {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Self::parse(env_value)
    }

    pub(crate) fn parse(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let positive = |key: &str| -> anyhow::Result<Option<u64>> {
            get(key)
                .map(|raw| {
                    raw.trim()
                        .parse::<u64>()
                        .ok()
                        .filter(|value| *value > 0)
                        .with_context(|| format!("{key} must be a positive whole number"))
                })
                .transpose()
        };
        let effort = match get("JEV_FALLBACK_REASONING") {
            Some(raw) => Effort::parse(&raw).map_err(|_| {
                anyhow::anyhow!("JEV_FALLBACK_REASONING must be none, low, medium or high")
            })?,
            None => Effort::Low,
        };
        Ok(Self {
            mode: FallbackMode::parse(get("JEV_FALLBACK").as_deref())?,
            model: get("JEV_FALLBACK_MODEL")
                .map(|raw| PinnedModel::parse(&raw))
                .transpose()?,
            effort,
            max_steps: positive("JEV_FALLBACK_MAX_STEPS")?.map_or(DEFAULT_STEPS, |v| v as usize),
            max_duration: Duration::from_secs(
                positive("JEV_FALLBACK_MAX_SECONDS")?.unwrap_or(DEFAULT_SECONDS),
            ),
            max_tokens: positive("JEV_FALLBACK_MAX_TOKENS")?.unwrap_or(DEFAULT_TOKENS),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn parse(pairs: &[(&str, &str)]) -> anyhow::Result<FallbackSettings> {
        let env = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<HashMap<_, _>>();
        FallbackSettings::parse(|key| env.get(key).cloned())
    }

    #[test]
    fn unset_is_auto_on_the_sessions_model_with_the_default_ceilings() {
        let settings = parse(&[]).unwrap();
        assert_eq!(settings, FallbackSettings::default());
        assert_eq!(settings.mode, FallbackMode::Auto);
        assert_eq!(settings.model, None);
        assert_eq!(settings.effort, Effort::Low);
        assert_eq!(
            (
                settings.max_steps,
                settings.max_duration,
                settings.max_tokens
            ),
            (20, Duration::from_secs(120), 400_000)
        );
    }

    #[test]
    fn every_setting_is_read_and_a_typo_is_refused() {
        let settings = parse(&[
            ("JEV_FALLBACK", " Handover "),
            ("JEV_FALLBACK_MODEL", "codex/gpt-6-sol"),
            ("JEV_FALLBACK_REASONING", "medium"),
            ("JEV_FALLBACK_MAX_STEPS", "8"),
            ("JEV_FALLBACK_MAX_SECONDS", "45"),
            ("JEV_FALLBACK_MAX_TOKENS", "90000"),
        ])
        .unwrap();
        assert_eq!(settings.mode, FallbackMode::Handover);
        assert_eq!(
            settings.model,
            Some(PinnedModel {
                provider: Some("codex".into()),
                model: "gpt-6-sol".into()
            })
        );
        assert_eq!(settings.effort, Effort::Medium);
        assert_eq!(settings.max_steps, 8);
        assert_eq!(settings.max_duration, Duration::from_secs(45));
        assert_eq!(settings.max_tokens, 90_000);
        let bare = parse(&[("JEV_FALLBACK_MODEL", "gpt-6-sol"), ("JEV_FALLBACK", "off")]).unwrap();
        assert_eq!(bare.model.unwrap().provider, None);
        assert_eq!(bare.mode, FallbackMode::Off);
        for (key, value) in [
            ("JEV_FALLBACK", "on"),
            ("JEV_FALLBACK_MODEL", "codex/"),
            ("JEV_FALLBACK_REASONING", "max"),
            ("JEV_FALLBACK_MAX_STEPS", "0"),
            ("JEV_FALLBACK_MAX_SECONDS", "soon"),
            ("JEV_FALLBACK_MAX_TOKENS", "-1"),
        ] {
            let error = parse(&[(key, value)]).unwrap_err().to_string();
            assert!(error.contains(key), "{key}: {error}");
        }
    }
}
