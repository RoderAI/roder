//! What the operator sets in the environment: the allowed origins
//! (`JEV_ALLOWED_ORIGINS`), the action budget (`JEV_MAX_ACTIONS`) and the
//! longest task (`JEV_MAX_SECONDS`). A call can narrow the origins and ask
//! for less time, never more. Two switches sit beside them: the
//! irreversible-action gate (`JEV_CONFIRM_IRREVERSIBLE`, off by default),
//! which a call cannot turn off, only authorize past, and cookie-banner
//! refusal (`JEV_REFUSE_COOKIE_BANNERS`, on unless set to 0).

use std::time::Duration;

use anyhow::Context;

use super::env_value;
use crate::scope::JevOriginScope;

/// What the operator set in the environment, which no call can widen.
#[derive(Debug, Clone)]
pub(crate) struct Ceilings {
    /// `JEV_ALLOWED_ORIGINS`.
    pub(crate) scope: JevOriginScope,
    /// `JEV_MAX_ACTIONS`: executed actions per run (twice as many decisions).
    pub(crate) max_actions: Option<usize>,
    /// `JEV_MAX_SECONDS`: the longest `timeout_seconds` may be.
    pub(crate) max_seconds: Option<Duration>,
    /// `JEV_CONFIRM_IRREVERSIBLE`: the irreversible-action gate.
    pub(crate) confirm_irreversible: bool,
    /// `JEV_REFUSE_COOKIE_BANNERS`: refuse cookie banners; on by default.
    pub(crate) refuse_cookie_banners: bool,
}

impl Default for Ceilings {
    /// Nothing set: no limits, the gate off and banner refusal on.
    fn default() -> Self {
        Self {
            scope: JevOriginScope::any(),
            max_actions: None,
            max_seconds: None,
            confirm_irreversible: false,
            refuse_cookie_banners: true,
        }
    }
}

impl Ceilings {
    /// Read from the environment. A malformed value fails every call rather
    /// than running without the operator's limit.
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            confirm_irreversible: switch(
                "JEV_CONFIRM_IRREVERSIBLE",
                env_value("JEV_CONFIRM_IRREVERSIBLE").as_deref(),
                false,
            )?,
            refuse_cookie_banners: switch(
                "JEV_REFUSE_COOKIE_BANNERS",
                env_value("JEV_REFUSE_COOKIE_BANNERS").as_deref(),
                true,
            )?,
            ..Self::parse(
                env_value("JEV_ALLOWED_ORIGINS").as_deref(),
                env_value("JEV_MAX_ACTIONS").as_deref(),
                env_value("JEV_MAX_SECONDS").as_deref(),
            )?
        })
    }

    pub(crate) fn parse(
        origins: Option<&str>,
        actions: Option<&str>,
        seconds: Option<&str>,
    ) -> anyhow::Result<Self> {
        let positive = |name: &str, raw: Option<&str>| -> anyhow::Result<Option<u64>> {
            raw.map(|raw| {
                raw.trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .with_context(|| format!("{name} must be a positive whole number"))
            })
            .transpose()
        };
        Ok(Self {
            scope: match origins {
                Some(list) => JevOriginScope::any()
                    .narrow_by_list(list)
                    .context("JEV_ALLOWED_ORIGINS")?,
                None => JevOriginScope::any(),
            },
            max_actions: positive("JEV_MAX_ACTIONS", actions)?.map(|value| value as usize),
            max_seconds: positive("JEV_MAX_SECONDS", seconds)?.map(Duration::from_secs),
            ..Self::default()
        })
    }
}

/// An on/off switch: `1`, `true`, `yes` or `on`, and `0`, `false`, `no` or
/// `off`, in any case; unset is `default`. Anything else fails every call,
/// so a typo cannot leave the gate off unnoticed.
pub(crate) fn switch(name: &str, raw: Option<&str>, default: bool) -> anyhow::Result<bool> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("{name} must be 1 or 0 (or true or false), not {raw:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_ceilings_are_refused_not_ignored() {
        assert!(Ceilings::parse(Some("example.com"), None, None).is_err());
        assert!(Ceilings::parse(None, Some("0"), None).is_err());
        assert!(Ceilings::parse(None, None, Some("soon")).is_err());
        let none = Ceilings::parse(None, None, None).unwrap();
        assert!(!none.scope.is_restricted());
        assert_eq!((none.max_actions, none.max_seconds), (None, None));
        assert!(!none.confirm_irreversible);
        assert!(none.refuse_cookie_banners);
    }

    #[test]
    fn switches_take_their_default_unless_set_and_a_typo_is_refused() {
        assert!(!switch("X", None, false).unwrap());
        assert!(switch("X", None, true).unwrap());
        for on in ["1", "true", " TRUE ", "yes", "on"] {
            assert!(switch("X", Some(on), false).unwrap(), "{on}");
        }
        for off in ["0", "false", "No", "off"] {
            assert!(!switch("X", Some(off), true).unwrap(), "{off}");
        }
        let error = switch("JEV_CONFIRM_IRREVERSIBLE", Some("ture"), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("JEV_CONFIRM_IRREVERSIBLE"), "{error}");
    }
}
