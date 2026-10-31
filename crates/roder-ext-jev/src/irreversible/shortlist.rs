//! Which observed actions the gate looks at, on the page's own evidence.
//!
//! A word list is never complete ("Place your order" names one, "Yes, I'm
//! sure" does not), so the shortlist reads more than the label: a link's
//! address, the heading or row a control sits under, and whether a button
//! posts its form. What it reads only decides what is asked; whether an
//! action commits stays the model's judgment. Three gates need no judgment
//! and follow fastbrowse's `safety.rs` (MIT): a playback control is never
//! asked about, a submit that sets a new password always stops, and a link
//! the page says saves a file stops when the goal rules out downloading.

use serde_json::Value;

/// Whole words in a click's label (or its address, heading or row) that
/// mark it as possibly committing something. The model judges each; this
/// only keeps the request small. "Sign" is left out: "Sign in" is on most
/// pages and would crowd out the shortlist. The words past the English ones
/// say the same in German, French, Spanish, Italian, Dutch and Portuguese,
/// so "Bestellen" is asked about as "Order" is.
pub(crate) const COMMIT_WORDS: [&str; 82] = [
    "pay",
    "payment",
    "buy",
    "purchase",
    "order",
    "checkout",
    "book",
    "reserve",
    "send",
    "submit",
    "post",
    "publish",
    "delete",
    "remove",
    "erase",
    "confirm",
    "transfer",
    "donate",
    "subscribe",
    "unsubscribe",
    "cancel",
    "apply",
    "deactivate",
    "withdraw",
    "terminate",
    "renew",
    "upgrade",
    "downgrade",
    "sell",
    "bid",
    "approve",
    // German.
    "bestellen",
    "kaufen",
    "bezahlen",
    "buchen",
    "senden",
    "absenden",
    "abschicken",
    "löschen",
    "entfernen",
    "bestätigen",
    "kündigen",
    "abonnieren",
    "abbestellen",
    "veröffentlichen",
    "überweisen",
    "spenden",
    "stornieren",
    "zahlungspflichtig",
    // French.
    "commander",
    "acheter",
    "payer",
    "réserver",
    "envoyer",
    "supprimer",
    "confirmer",
    "publier",
    "annuler",
    "désabonner",
    "résilier",
    // Spanish and Portuguese.
    "comprar",
    "pagar",
    "reservar",
    "enviar",
    "eliminar",
    "borrar",
    "excluir",
    "confirmar",
    "publicar",
    "cancelar",
    // Italian.
    "acquista",
    "ordina",
    "paga",
    "prenota",
    "invia",
    "elimina",
    "conferma",
    "pubblica",
    // Dutch.
    "kopen",
    "betalen",
    "afrekenen",
    "verwijderen",
];

/// A label that agrees to whatever the dialog around it asked ("Yes, I'm
/// sure"): only the heading it sits under says what.
const AFFIRMATIONS: [&str; 2] = ["yes", "sure"];

/// Playback controls: what they do the same control undoes, with no effect
/// off the page (fastbrowse's `REVERSIBLE_TOGGLE`).
const PLAYBACK: [&str; 6] = ["play", "pause", "resume", "unmute", "mute", "replay"];

/// Nouns that turn a pause into a commitment ("Pause my subscription"),
/// fastbrowse's `COMMITTED_NOUN`.
const COMMITTED_NOUNS: [&str; 22] = [
    "subscription",
    "membership",
    "plan",
    "billing",
    "payment",
    "invoice",
    "order",
    "shipment",
    "delivery",
    "account",
    "profile",
    "job",
    "project",
    "campaign",
    "listing",
    "ad",
    "deploy",
    "server",
    "instance",
    "customer",
    "contract",
    "policy",
];

/// The verbs of a label that changes a password ("Reset your password").
const CHANGE_VERBS: [&str; 5] = ["change", "reset", "update", "replace", "set"];

/// A button that only shows or hides what a field holds.
const VISIBILITY: [&str; 3] = ["show", "hide", "reveal"];

/// File types a browser saves rather than shows (fastbrowse's
/// `DOWNLOADED_SUFFIXES`): a page, an image or a PDF is rendered, an
/// archive, an installer, a disk image or a spreadsheet goes to disk.
const DOWNLOADED_SUFFIXES: [&str; 33] = [
    "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "exe", "msi", "dmg", "pkg", "deb",
    "rpm", "apk", "appimage", "iso", "img", "jar", "torrent", "csv", "tsv", "xls", "xlsx", "ods",
    "doc", "docx", "odt", "ppt", "pptx", "odp", "epub",
];

/// The lower-cased whole words of `text`.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

fn has_word(text: &str, list: &[&str]) -> bool {
    words(text).any(|word| list.contains(&word.as_str()))
}

fn text<'a>(action: &'a Value, key: &str) -> &'a str {
    action[key].as_str().unwrap_or_default()
}

/// Whether the label holds a commitment word as a whole word, in any case.
pub(crate) fn names_commitment(label: &str) -> bool {
    has_word(label, &COMMIT_WORDS)
}

/// Where the control sits: the row or card that tells it from its twins,
/// else the nearest heading before it.
pub(crate) fn place(action: &Value) -> &str {
    match text(action, "context") {
        "" => text(action, "section"),
        context => context,
    }
}

/// The path and query of a link's address, as the snapshot gives it (a
/// path on this site, or a host and path on another).
fn link_path(action: &Value) -> &str {
    let href = text(action, "href");
    if href.starts_with('/') {
        return href;
    }
    href.split_once('/').map_or("", |(_, path)| path)
}

/// The last segment of a link's path ("unsubscribe" of
/// `/mail/unsubscribe?token=…`): what the link does, where the segments
/// before it and the query name what it is about (`/book/dune`,
/// `/search?q=buy+shoes`).
fn link_action(action: &Value) -> &str {
    let path = link_path(action);
    let path = path.split(['?', '#']).next().unwrap_or_default();
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
}

/// The most words a link's last segment may hold and still say what the
/// link does (`unsubscribe`, `place-order.php`) rather than name an article
/// about it (`how-to-cancel-a-subscription`).
const ACTION_WORDS: usize = 3;

fn link_names_commitment(action: &Value) -> bool {
    let segment = link_action(action);
    let stem = segment.rsplit_once('.').map_or(segment, |(stem, _)| stem);
    words(stem).count() <= ACTION_WORDS && names_commitment(stem)
}

/// How strongly an action's own evidence says it may commit. The stronger
/// are asked about first, so a page full of weak ones cannot crowd them out.
/// Only a [`Tier::Named`] action pushed past the question cap fails closed,
/// as every gated action did before the gate read more than the label; the
/// others are evidence the gate reads to ask more, never a reason to stop
/// what it had no room to ask about (a list of twenty "Add to cart" forms
/// that each post).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tier {
    /// An Enter, or a click whose label names a commitment.
    Named,
    /// A link whose address names one (`/unsubscribe`).
    Linked,
    /// A click whose heading or row names one, that agrees to what a
    /// dialog asked, or that posts its form.
    Placed,
}

/// The tier of an action the gate applies to; `None` for one it never
/// asks about.
pub(crate) fn tier(action: &Value) -> Option<Tier> {
    match action["kind"].as_str() {
        Some("enter") => Some(Tier::Named),
        Some("click") if reversible_toggle(action) => None,
        Some("click") => {
            let label = text(action, "label");
            if names_commitment(label) {
                Some(Tier::Named)
            } else if link_names_commitment(action) {
                Some(Tier::Linked)
            } else if has_word(label, &AFFIRMATIONS)
                // A pause that is not playback ("Pause my subscription").
                || has_word(label, &PLAYBACK)
                || names_commitment(place(action))
                || (action["role"] == "button" && action["submits"]["method"] == "post")
            {
                Some(Tier::Placed)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Whether activating this click only flips playback: the label names play,
/// pause, mute and the like, and neither it nor where it sits names
/// something a pause would commit (a subscription, a shipment, a campaign).
pub(crate) fn reversible_toggle(action: &Value) -> bool {
    let label = text(action, "label");
    action["kind"] == "click"
        && has_word(label, &PLAYBACK)
        && !names_commitment(label)
        && !has_word(label, &COMMITTED_NOUNS)
        && !has_word(place(action), &COMMITTED_NOUNS)
}

/// Whether the page says this click saves a file, before anything is
/// clicked: a `download` attribute, or a link to a file type a browser
/// saves. An icon-only download link carries nothing else, in any language.
pub(crate) fn starts_download(action: &Value) -> bool {
    if action["kind"] != "click" {
        return false;
    }
    if action["download"] == true {
        return true;
    }
    link_action(action)
        .rsplit_once('.')
        .is_some_and(|(stem, suffix)| {
            !stem.is_empty() && DOWNLOADED_SUFFIXES.contains(&suffix.to_lowercase().as_str())
        })
}

/// `change|reset|update|replace|set`, then `password` later on its line.
fn names_password_change(label: &str) -> bool {
    label.lines().any(|line| {
        let words = words(line).collect::<Vec<_>>();
        words
            .iter()
            .position(|word| CHANGE_VERBS.contains(&word.as_str()))
            .is_some_and(|verb| words[verb + 1..].iter().any(|word| word == "password"))
    })
}

/// Whether this action submits a new password: a button (not a link) or an
/// Enter in a form that holds a field for a new password, or a button whose
/// label says it changes one. A link to the settings page that does so is
/// only navigation, and a button that shows the password changes nothing.
pub(crate) fn changes_credentials<'a>(
    actions: impl IntoIterator<Item = &'a Value>,
    action: &Value,
) -> bool {
    let label = text(action, "label");
    match action["kind"].as_str() {
        Some("click") => {
            if action["role"] != "button" || !text(action, "href").is_empty() {
                return false;
            }
            if has_word(label, &VISIBILITY) {
                return false;
            }
        }
        Some("enter") => {}
        _ => return false,
    }
    let in_form = actions.into_iter().any(|other| {
        crate::wall::same_form(action, other) && crate::wall::sets_new_password(other)
    });
    in_form || names_password_change(label)
}

#[cfg(test)]
mod tests;
