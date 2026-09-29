//! Jev's rules on the full browser tools, so nothing Jev stops for is
//! weakened when another driver takes its tab: the operator's allowed
//! origins, the irreversible-action gate, the secrets typed in the session,
//! and Jev's own reading of an access block.
//!
//! The gate has no model question here, so it stops at every control its
//! shortlist names instead of asking about each: a click whose label holds
//! one of [`crate::irreversible::COMMIT_WORDS`], a form's submit button once
//! a password or code is filled in it, and Enter in a form whose submit
//! control names a commitment or that holds a filled password or code. That
//! is stricter than Jev, whose model can clear a shortlisted control; it
//! does not stop a plain Enter in a search box.

use std::sync::{Arc, Mutex};

use roder_ext_chrome::direct::{DirectGuard, GateAction, GateQuery, PageFacts};

use crate::block;
use crate::irreversible::names_commitment;
use crate::scope::JevOriginScope;
use crate::secret::Secrets;

pub(crate) struct JevGuard {
    scope: JevOriginScope,
    /// `JEV_CONFIRM_IRREVERSIBLE`.
    gate: bool,
    /// `JEV_REFUSE_COOKIE_BANNERS`: a banner may be refused; off, none is
    /// touched.
    banners: bool,
    /// The session's typed secrets, and any the fallback types.
    secrets: Arc<Mutex<Secrets>>,
}

impl JevGuard {
    pub(crate) fn new(scope: JevOriginScope, gate: bool, banners: bool, secrets: Secrets) -> Self {
        Self {
            scope,
            gate,
            banners,
            secrets: Arc::new(Mutex::new(secrets)),
        }
    }

    /// Remember a secret the fallback typed, so it is scrubbed from what is
    /// read from now on.
    pub(crate) fn remember(&self, secret: &str) {
        if let Ok(mut secrets) = self.secrets.lock() {
            secrets.remember(secret);
        }
    }

    /// Every secret known now, the session's and the fallback's.
    pub(crate) fn secrets(&self) -> Secrets {
        self.secrets
            .lock()
            .map(|secrets| secrets.clone())
            .unwrap_or_default()
    }
}

impl DirectGuard for JevGuard {
    fn outside(&self, url: &str) -> Option<String> {
        (!self.scope.allows(url)).then(|| self.scope.outside(url))
    }

    /// A cookie or consent banner is Jev's banner refusal's: with it on, a
    /// press there may only refuse (Jev never accepts or opens settings);
    /// with it off, nothing there is pressed.
    fn refuses(&self, query: &GateQuery) -> Option<String> {
        if !query.consent {
            return None;
        }
        match (self.banners, refuses_consent(&query.label)) {
            (true, true) => None,
            (true, false) => Some(format!(
                "\"{}\" sits in a cookie or consent banner, where only a refusal (such as \
                 \"Reject all\") is pressed: nothing is accepted on the user's behalf; nothing \
                 was pressed.",
                query.label
            )),
            (false, _) => Some(
                "The operator turned cookie-banner refusal off, so nothing in a cookie or \
                 consent banner is pressed; nothing was pressed."
                    .to_string(),
            ),
        }
    }

    fn confirm(&self, query: &GateQuery) -> Option<String> {
        if !self.gate {
            return None;
        }
        if query.frame {
            return Some(format!(
                "The press lands inside a frame of another site ({}), where Roder cannot see \
                 which control it would press, and the operator's irreversible-action gate \
                 stops before anything it cannot check; nothing was pressed. Ask the user to \
                 confirm that exact step.",
                query.label
            ));
        }
        let shortlisted = match query.action {
            GateAction::Click => {
                names_commitment(&query.label) || (query.submit && query.secret_form)
            }
            GateAction::Enter => {
                query.secret_form
                    || query
                        .form_labels
                        .iter()
                        .any(|label| names_commitment(label))
            }
        };
        shortlisted.then(|| {
            let what = match query.action {
                GateAction::Click => format!("\"{}\"", query.label),
                GateAction::Enter => "Enter in this form".to_string(),
            };
            format!(
                "{what} may make a purchase, payment, send, publish, delete or other change that \
                 cannot be undone, and the operator's irreversible-action gate stops before it; \
                 nothing was pressed. Ask the user to confirm that exact step."
            )
        })
    }

    fn scrub(&self, text: &str) -> String {
        self.secrets
            .lock()
            .map(|secrets| secrets.scrub(text))
            .unwrap_or_else(|_| text.to_string())
    }

    fn refused(&self, facts: &PageFacts) -> Option<String> {
        block::classify(
            facts.http_status,
            &facts.url,
            &facts.title,
            &facts.text,
            facts.controls,
        )
        .map(|block| self.scrub(&block.reason()))
    }
}

/// Whether a banner button's label refuses consent: a refusal word and no
/// word that accepts, allows or opens settings.
fn refuses_consent(label: &str) -> bool {
    const REFUSE: [&str; 13] = [
        "reject",
        "decline",
        "refuse",
        "deny",
        "necessary",
        "essential",
        "ablehnen",
        "refuser",
        "rechazar",
        "rifiuta",
        "weigeren",
        "rejeitar",
        "recusar",
    ];
    const OTHER: [&str; 12] = [
        "accept",
        "agree",
        "allow",
        "ok",
        "okay",
        "yes",
        "got",
        "manage",
        "settings",
        "preferences",
        "customize",
        "customise",
    ];
    let words = label
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    words.iter().any(|word| REFUSE.contains(&word.as_str()))
        && !words.iter().any(|word| OTHER.contains(&word.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(action: GateAction, label: &str) -> GateQuery {
        GateQuery {
            action,
            label: label.into(),
            role: Some("button".into()),
            submit: false,
            form_labels: Vec::new(),
            secret_form: false,
            consent: false,
            frame: false,
        }
    }

    /// Jev never accepts a cookie banner, and with refusal off touches none;
    /// the fallback inherits both.
    #[test]
    fn a_consent_banner_may_only_be_refused_and_not_at_all_with_refusal_off() {
        let banner = |label: &str| GateQuery {
            consent: true,
            ..query(GateAction::Click, label)
        };
        let on = JevGuard::new(JevOriginScope::any(), false, true, Secrets::default());
        assert!(on.refuses(&banner("Reject all")).is_none());
        assert!(on.refuses(&banner("Only necessary cookies")).is_none());
        let accept = on.refuses(&banner("Accept all")).unwrap();
        assert!(accept.contains("only a refusal"), "{accept}");
        assert!(on.refuses(&banner("Manage preferences")).is_some());
        assert!(on.refuses(&banner("OK, reject")).is_some());
        assert!(
            on.refuses(&query(GateAction::Click, "Accept all"))
                .is_none()
        );
        let off = JevGuard::new(JevOriginScope::any(), false, false, Secrets::default());
        let why = off.refuses(&banner("Reject all")).unwrap();
        assert!(why.contains("cookie-banner refusal off"), "{why}");
    }

    #[test]
    fn the_gate_is_the_operators_and_stops_at_its_shortlist() {
        let off = JevGuard::new(JevOriginScope::any(), false, true, Secrets::default());
        assert_eq!(off.confirm(&query(GateAction::Click, "Pay now")), None);
        let on = JevGuard::new(JevOriginScope::any(), true, true, Secrets::default());
        let why = on.confirm(&query(GateAction::Click, "Pay now")).unwrap();
        assert!(
            why.contains("\"Pay now\"") && why.contains("nothing was pressed"),
            "{why}"
        );
        assert!(on.confirm(&query(GateAction::Click, "Reserve")).is_some());
        assert!(on.confirm(&query(GateAction::Click, "Close")).is_none());
        // A plain Enter in a search box goes; one in a form that pays or
        // holds a filled password does not.
        assert!(on.confirm(&query(GateAction::Enter, "Search")).is_none());
        let mut paying = query(GateAction::Enter, "Card number");
        paying.form_labels = vec!["Place order".into()];
        assert!(on.confirm(&paying).is_some());
        let mut signing_in = query(GateAction::Click, "Continue");
        signing_in.submit = true;
        signing_in.secret_form = true;
        assert!(on.confirm(&signing_in).is_some());
        // A press into another site's frame cannot be checked: stopped.
        let framed = GateQuery {
            frame: true,
            ..query(GateAction::Click, "https://widgets.example")
        };
        assert!(
            on.confirm(&framed)
                .unwrap()
                .contains("frame of another site")
        );
        assert!(off.confirm(&framed).is_none());
    }

    #[test]
    fn scope_secrets_and_access_blocks_are_jevs() {
        let scope = JevOriginScope::any()
            .narrow(&["https://*.example.com"])
            .unwrap();
        let mut secrets = Secrets::default();
        secrets.remember("hunter22");
        let guard = JevGuard::new(scope, false, true, secrets);
        assert!(guard.outside("https://shop.example.com/x").is_none());
        let why = guard.outside("https://evil.test/").unwrap();
        assert!(why.contains("outside the allowed origins"), "{why}");
        assert_eq!(guard.scrub("pw hunter22 and 9999"), "pw [secret] and 9999");
        guard.remember("9999");
        assert_eq!(
            guard.scrub("pw hunter22 and 9999"),
            "pw [secret] and [secret]"
        );
        assert_eq!(guard.secrets().len(), 2);
        let wall = PageFacts {
            url: "https://shop.example.com/".into(),
            title: "Access Denied".into(),
            text: "Access Denied".into(),
            http_status: Some(403),
            controls: 0,
        };
        assert!(
            guard
                .refused(&wall)
                .unwrap()
                .contains("refused automated access")
        );
        let fine = PageFacts {
            http_status: Some(200),
            controls: 12,
            title: "Menu".into(),
            ..wall
        };
        assert_eq!(guard.refused(&fine), None);
    }
}
