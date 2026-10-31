//! The consult's pure rules: what may be named, repeated and consulted.

use super::*;

#[test]
fn a_consult_never_names_a_banner_choice_on_a_page_about_cookies() {
    let page = |text: &str| json!({"text": text});
    let button = |label: &str| json!({"kind": "click", "label": label});
    let cookies = consent_page(&page("We use cookies to improve the site."));
    assert!(cookies && consent_page(&page("Manage your CONSENT")));
    assert!(!consent_page(&page("Feed item 1")));
    for label in [
        "Accept all",
        "Reject all",
        "No thanks",
        "Only necessary",
        "Allow cookies",
    ] {
        assert!(!nameable(&button(label), cookies), "{label}");
        // Off a page about cookies, such a word is only a word.
        assert!(nameable(&button(label), false), "{label}");
    }
    assert!(nameable(&button("Show more"), cookies));
    assert!(nameable(&button("Accepted orders"), cookies));
    // Waits and document reads are never named.
    for kind in ["wait", "read_document"] {
        assert!(!nameable(&json!({"kind": kind, "label": "x"}), false));
    }
}

#[test]
fn only_a_repeat_that_commits_and_adds_nothing_may_be_endorsed() {
    let button = |label: &str| json!({"kind": "click", "label": label});
    for label in ["Load more", "Next page", "Show more results"] {
        assert!(repeatable(&button(label)), "{label}");
    }
    for label in ["Add to cart", "Delete application", "Add", "Upvote", "Send"] {
        assert!(!repeatable(&button(label)), "{label}");
    }
    let posts = json!({"kind": "click", "label": "More", "submits": {"method": "post"}});
    assert!(!repeatable(&posts));
    let gets = json!({"kind": "click", "label": "More", "submits": {"method": "get"}});
    assert!(repeatable(&gets));
}

#[test]
fn only_steps_that_act_are_consulted_and_named_kinds_map_to_operations() {
    for (kind, acting, operation) in [
        ("click", true, "CLICK"),
        ("fill", true, "TYPE_TEXT"),
        ("select", true, "SELECT"),
        ("enter", true, "PRESS_ENTER"),
        ("scroll", false, "SCROLL"),
        ("key", false, "PRESS_KEY"),
    ] {
        let action = json!({"kind": kind, "id": "e1"});
        assert_eq!(acts(&action), acting, "{kind}");
        assert_eq!(operation_of(&action), operation);
    }
    let swapped = directed(&json!({"kind": "click", "id": "e7"}), json!({}));
    assert_eq!(swapped.choice, "e7");
    // No answers and no confidence: every gate holds it to the most.
    assert_eq!(swapped.call_confidence(), 0.0);
    assert!(swapped.irreversible.is_none() && swapped.downloads_forbidden.is_none());
}
