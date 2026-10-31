//! Sign-in walls: a page that wants credentials the run does not hold.
//!
//! A run that reaches a sign-in it cannot complete used to end `needs_input`
//! (the goal gave no password) or `blocked` (the model gave up on the form),
//! and a caller told either narrowed the task or retried, where only the
//! user can help. Such a run now ends [`crate::JevStatus::NeedsLogin`] with
//! a typed [`JevWall`], so a caller routes on what the wall wants: a one-time
//! code it may fetch, a password only the user has, or a sign-in to make.
//! Ported from fastbrowse's `credential_field` and `wall_kind` (MIT).
//!
//! The page's own evidence decides, never a guess: a password or one-time
//! code field (a field for a new password is not one), a page that names a
//! sign-in in its title or a heading, and its controls that offer one.

use reqwest::Url;
use serde::Serialize;
use serde_json::Value;

/// What a sign-in wall asks of the person who has to get the run past it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum JevWallKind {
    /// A password field: the account's credentials.
    Password,
    /// Only one-time-code fields: a code the site sent by email, text or an
    /// authenticator app.
    OneTimeCode,
    /// A sign-in to make (a provider, single sign-on, an HTTP 401) and no
    /// password or code field yet.
    SignIn,
}

/// The sign-in wall a `needs_login` run stopped at, typed so a caller can
/// route it without parsing `stopped_because`. Page text, as untrusted as
/// the rest, with typed secrets scrubbed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JevWall {
    pub kind: JevWallKind,
    /// The origin of the page that showed it.
    pub origin: String,
    /// The labels of the password and code fields it asked for, at most
    /// eight.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
}

impl JevWall {
    /// Why the run stopped, ahead of what stopped it.
    pub(crate) fn reason(&self, stopped_because: Option<&str>) -> String {
        let wants = match self.kind {
            JevWallKind::Password => "a password",
            JevWallKind::OneTimeCode => "a one-time code",
            JevWallKind::SignIn => "a sign-in",
        };
        let fields = match self.fields.as_slice() {
            [] => String::new(),
            fields => format!(
                " ({})",
                fields
                    .iter()
                    .map(|field| format!("{field:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let mut reason = format!(
            "Sign-in required at {}: the page asks for {wants}{fields}",
            self.origin
        );
        if let Some(why) = stopped_because.filter(|why| !why.is_empty()) {
            reason.push_str(". ");
            reason.push_str(why);
        }
        reason
    }
}

/// The most field labels a wall names.
const MAX_FIELDS: usize = 8;
const LABEL_CHARS: usize = 80;

/// Which part of a sign-in a field takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Credential {
    Password,
    OneTimeCode,
}

/// Words that make a password field one for a new password: a sign-up or
/// a password change, not a sign-in.
const NEW_SECRET: [&str; 10] = [
    "new",
    "confirm",
    "confirmation",
    "repeat",
    "reenter",
    "re-enter",
    "retype",
    "re-type",
    "choose",
    "create",
];

/// What credential a fill action takes, if it is a sign-in field.
fn credential(action: &Value) -> Option<Credential> {
    if action["kind"] != "fill" {
        return None;
    }
    match action["input_type"].as_str()? {
        "one-time-code" => Some(Credential::OneTimeCode),
        "password" => {
            let label = action["label"].as_str().unwrap_or_default().to_lowercase();
            let new = label
                .split(|c: char| !c.is_alphanumeric() && c != '-')
                .any(|word| NEW_SECRET.contains(&word));
            (!new).then_some(Credential::Password)
        }
        _ => None,
    }
}

/// Phrases that name a sign-in, matched on whole words of lower-cased text
/// with punctuation as spaces.
const SIGN_IN: [&str; 23] = [
    "sign in",
    "signin",
    "sign on",
    "log in",
    "login",
    "log on",
    "logon",
    "authenticate",
    "authentication",
    "verify",
    "verification",
    "two factor",
    "2fa",
    "one time",
    "sso",
    "saml",
    "single sign on",
    "anmelden",
    "einloggen",
    "connexion",
    "se connecter",
    "iniciar sesion",
    "iniciar sesión",
];

/// What a control offering a sign-in through a provider says ("Continue
/// with Google"). Only a control's label counts: a heading that says it
/// ("Continue with checkout") names no sign-in.
const PROVIDER: [&str; 1] = ["continue with"];

/// Whether `text` names a sign-in.
fn names_sign_in(text: &str) -> bool {
    has_phrase(text, &SIGN_IN)
}

/// Whether a control's label offers a sign-in.
fn offers_sign_in(label: &str) -> bool {
    names_sign_in(label) || has_phrase(label, &PROVIDER)
}

/// Whether `text`, as lower-cased words, holds one of `phrases`.
fn has_phrase(text: &str, phrases: &[&str]) -> bool {
    let words = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let padded = format!(" {words} ");
    phrases
        .iter()
        .any(|phrase| padded.contains(&format!(" {phrase} ")))
}

fn label(action: &Value) -> &str {
    action["label"].as_str().unwrap_or_default()
}

fn actions(observation: &Value) -> impl Iterator<Item = &Value> {
    observation["actions"].as_array().into_iter().flatten()
}

/// The origin of the page, or its address when it has none.
fn origin(observation: &Value) -> String {
    let url = observation["url"].as_str().unwrap_or_default();
    match Url::parse(url).map(|url| url.origin()) {
        Ok(origin) if origin.is_tuple() => origin.ascii_serialization(),
        _ => url.to_string(),
    }
}

/// The wall made of these credential fields: a password if any is one, a
/// one-time code if all are, else a sign-in.
fn wall(observation: &Value, fields: &[&Value]) -> JevWall {
    let kinds = fields.iter().filter_map(|field| credential(field));
    let kinds = kinds.collect::<Vec<_>>();
    let kind = if kinds.contains(&Credential::Password) {
        JevWallKind::Password
    } else if !kinds.is_empty() {
        JevWallKind::OneTimeCode
    } else {
        JevWallKind::SignIn
    };
    let mut labels: Vec<String> = Vec::new();
    for field in fields {
        let label = label(field).chars().take(LABEL_CHARS).collect::<String>();
        if !label.is_empty() && !labels.contains(&label) && labels.len() < MAX_FIELDS {
            labels.push(label);
        }
    }
    JevWall {
        kind,
        origin: origin(observation),
        fields: labels,
    }
}

/// The wall a page shows, on its own evidence: an empty password or code
/// field on a page that names a sign-in (its title, a heading, or a control
/// such as "Sign in" or "Verify"), or a page whose title or a heading names
/// a sign-in and that offers a control to make one ("Continue with Google",
/// "Sign in with SSO", or "Sign in" under a password typed and refused).
/// A page that merely links to a sign-in in its header names none in its
/// title or headings, and a filled password field on it is not a wall.
pub(crate) fn shown(observation: &Value, headings: &[String]) -> Option<JevWall> {
    let announced = names_sign_in(observation["title"].as_str().unwrap_or_default())
        || headings.iter().any(|heading| names_sign_in(heading));
    let offered = actions(observation).any(|action| {
        matches!(action["kind"].as_str(), Some("click" | "enter")) && offers_sign_in(label(action))
    });
    let fields = actions(observation)
        .filter(|action| credential(action).is_some() && !in_sign_up(observation, action))
        .collect::<Vec<_>>();
    let empty = fields.iter().any(|field| field["filled"] != true);
    ((empty && (announced || offered)) || (announced && offered))
        .then(|| wall(observation, &fields))
}

/// Whether a fill action is a password field for a new password.
fn sets_new_password(action: &Value) -> bool {
    action["kind"] == "fill" && action["input_type"] == "password" && credential(action).is_none()
}

/// Whether `field` sits in a form that also sets a new password: a sign-up
/// or a password change, whose plain "Password" is one to choose.
fn in_sign_up(observation: &Value, field: &Value) -> bool {
    field["form"]["id"].as_str().is_some_and(|form| {
        actions(observation)
            .any(|action| action["form"]["id"].as_str() == Some(form) && sets_new_password(action))
    })
}

/// The wall behind a field the run had no value for: the field is a
/// password or code field, or sits in the same form as one. Any other
/// missing value is the caller's to give, and stays `needs_input`; so is
/// any value of a form that also sets a new password (a sign-up, whose
/// "Password" beside "Confirm password" is one to choose, not to sign in
/// with).
pub(crate) fn behind(observation: &Value, missing: &Value) -> Option<JevWall> {
    if in_sign_up(observation, missing) {
        return None;
    }
    let fields = actions(observation)
        .filter(|action| credential(action).is_some())
        .collect::<Vec<_>>();
    if credential(missing).is_some() {
        let mut fields = fields;
        if !fields.iter().any(|field| field["id"] == missing["id"]) {
            fields.insert(0, missing);
        }
        return Some(wall(observation, &fields));
    }
    let form = missing["form"]["id"].as_str()?;
    let in_form = fields
        .into_iter()
        .filter(|field| field["form"]["id"].as_str() == Some(form))
        .collect::<Vec<_>>();
    (!in_form.is_empty()).then(|| wall(observation, &in_form))
}

/// The wall of a page that answered HTTP 401: its own fields when it shows
/// some, else a sign-in.
pub(crate) fn unauthorized(observation: &Value) -> JevWall {
    let fields = actions(observation)
        .filter(|action| credential(action).is_some())
        .collect::<Vec<_>>();
    wall(observation, &fields)
}

#[cfg(test)]
mod tests;
