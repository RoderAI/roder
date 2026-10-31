//! Unit tests of what the gate reads from an observed action.

use serde_json::{Value, json};

use super::*;

fn click(label: &str, extra: Value) -> Value {
    let mut action = json!({"kind": "click", "role": "button", "label": label});
    for (key, value) in extra.as_object().unwrap() {
        action[key] = value.clone();
    }
    action
}

fn link(label: &str, href: &str) -> Value {
    json!({"kind": "click", "role": "link", "label": label, "href": href})
}

#[test]
fn commitment_words_match_whole_words_in_any_case_and_in_other_languages() {
    for label in [
        "Pay now",
        "PAY",
        "Payment details",
        "Place your order",
        "Delete account",
        "Send message",
        "Unsubscribe",
        "Confirm & book",
        "Submit application",
        "Bestellen",
        "Zahlungspflichtig bestellen",
        "Konto löschen",
        "Commander",
        "Comprar ahora",
        "Deactivate account",
    ] {
        assert!(names_commitment(label), "{label}");
    }
    for label in [
        "Add to cart",
        "Display options",
        "Sign in",
        "Orders",
        "Paypal",
        "Next page",
        "Bestellung ansehen",
        "",
    ] {
        assert!(!names_commitment(label), "{label}");
    }
}

#[test]
fn a_label_or_the_last_segment_of_a_link_names_a_commitment_first() {
    assert_eq!(
        tier(&json!({"kind": "enter", "label": "Message"})),
        Some(Tier::Named)
    );
    assert_eq!(tier(&click("Pay now", json!({}))), Some(Tier::Named));
    assert_eq!(tier(&click("Bestellen", json!({}))), Some(Tier::Named));
    // A one-click unsubscribe is a link whose label says nothing.
    let stop = link("Stop these emails", "/mail/unsubscribe?token=abc");
    assert_eq!(tier(&stop), Some(Tier::Linked));
    let elsewhere = link("Stop these emails", "lists.example.com/u/unsubscribe");
    assert_eq!(tier(&elsewhere), Some(Tier::Linked));
    let order = link("Continue", "/checkout/place-order.php?step=3");
    assert_eq!(tier(&order), Some(Tier::Linked));
    // A label that names one ranks first, whatever the address.
    assert_eq!(
        tier(&link("Unsubscribe", "/mail/unsubscribe")),
        Some(Tier::Named)
    );
    // What a link is about is not what it does.
    for href in [
        "/book/dune",
        "/search?q=buy+shoes",
        "/post/how-to/",
        "pay.example.com/",
        "#unsubscribe",
        "/blog/how-to-cancel-your-subscription",
    ] {
        assert_eq!(tier(&link("Read more", href)), None, "{href}");
    }
    // Typing, choosing and waiting commit nothing by themselves.
    for kind in ["fill", "select", "wait"] {
        assert_eq!(
            tier(&json!({"kind": kind, "label": "Order note"})),
            None,
            "{kind}"
        );
    }
}

#[test]
fn where_a_control_sits_and_what_its_form_posts_name_a_commitment_second() {
    let sure = click("Yes, I'm sure", json!({"section": "Are you sure?"}));
    assert_eq!(tier(&sure), Some(Tier::Placed));
    let under = click("Continue", json!({"section": "Delete this project?"}));
    assert_eq!(tier(&under), Some(Tier::Placed));
    // A twin's row counts before the heading.
    let row = click(
        "Go",
        json!({"context": "Cancel booking #4", "section": "Bookings"}),
    );
    assert_eq!(tier(&row), Some(Tier::Placed));
    let posts = click(
        "Save",
        json!({"submits": {"method": "post", "action": "/s", "text": ""}}),
    );
    assert_eq!(tier(&posts), Some(Tier::Placed));
    let gets = click(
        "Search",
        json!({"submits": {"method": "get", "action": "/q", "text": ""}}),
    );
    assert_eq!(tier(&gets), None);
    // Only a button posts its form.
    let field = json!({"kind": "click", "role": "textbox", "label": "Open Name",
        "submits": {"method": "post", "action": "/s", "text": ""}});
    assert_eq!(tier(&field), None);
    assert_eq!(
        tier(&click("Continue", json!({"section": "Shipping"}))),
        None
    );
}

#[test]
fn playback_controls_are_reversible_and_never_asked_about() {
    for label in ["Play", "Pause", "Resume", "Mute", "Unmute", "Replay video"] {
        let control = click(label, json!({"section": "Publish your podcast"}));
        assert!(reversible_toggle(&control), "{label}");
        assert_eq!(tier(&control), None, "{label}");
    }
    // A pause that commits something is asked about.
    for (label, section) in [
        ("Pause my subscription", ""),
        ("Pause billing", ""),
        ("Play the demo with your order", ""),
        ("Pause", "Your membership"),
    ] {
        let control = click(label, json!({"section": section}));
        assert!(!reversible_toggle(&control), "{label}");
        assert!(tier(&control).is_some(), "{label}");
    }
}

#[test]
fn a_download_is_read_from_the_page_not_the_label() {
    for (href, download, expected) in [
        ("/files/addon-2.1.0.zip?sig=1", false, true),
        ("cdn.example.com/releases/Setup.EXE", false, true),
        ("/reports/q3.csv", false, true),
        ("/export", true, true),
        ("/releases/2.1.0", false, false),
        ("/guide.pdf", false, false),
        ("downloads.zip/", false, false),
        ("/files/.zip", false, false),
    ] {
        let mut control = link("Get it", href);
        if download {
            control["download"] = json!(true);
        }
        assert_eq!(starts_download(&control), expected, "{href}");
    }
    let mut enter = link("Get it", "/a.zip");
    enter["kind"] = json!("enter");
    assert!(!starts_download(&enter));
}

fn new_password(form: u64) -> Value {
    json!({"kind": "fill", "role": "textbox", "label": "New password",
        "input_type": "password", "form": {"id": form}})
}

#[test]
fn a_new_password_is_recognized_from_the_form_or_the_label() {
    let save = click("Save", json!({"form": {"id": 7}}));
    let offered = [new_password(7), save.clone()];
    assert!(changes_credentials(&offered, &save));
    // Another form's button, a field's Enter in the form, and the label.
    assert!(!changes_credentials(&[new_password(8)], &save));
    let enter = json!({"kind": "enter", "role": "textbox", "label": "New password",
        "form": {"id": 7}});
    assert!(changes_credentials(&offered, &enter));
    assert!(changes_credentials(
        &[],
        &click("Reset your password", json!({}))
    ));
    assert!(!changes_credentials(
        &[],
        &click("Reset\npassword", json!({}))
    ));
    assert!(!changes_credentials(&[], &click("Save", json!({}))));
    // A link to the settings page, a button that shows the password, and a
    // sign-in's own password field do not change it.
    assert!(!changes_credentials(
        &[],
        &link("Change password", "/settings/password")
    ));
    let show = click("Show password", json!({"form": {"id": 7}}));
    assert!(!changes_credentials(&offered, &show));
    let current = json!({"kind": "fill", "role": "textbox", "label": "Password",
        "input_type": "password", "form": {"id": 7}});
    assert!(!changes_credentials(&[current], &save));
}
