//! What covers a target is named, with what it offers to press: in the
//! step's record, in the digest the caller reads, and in the fallback's
//! brief. Without the name every refusal read alike ("another element covers
//! the target"), and a caller or a fallback could not tell a sale popup it
//! should close from a menu it opened itself. A band fixed to the window
//! (`anchored-form.html`) is not a cover to report: the target is scrolled
//! clear of it, or pressed from the keyboard under it.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::fallback_script::ScriptedFallback;
use super::fallback_tests::{ceilings, jev};
use super::scripted::find;
use super::sessions::{call_falling_back, test_sessions};
use crate::engine::{Covered, StaleObservation};
use crate::fallback::FallbackMode;
use crate::report::tool_text;

/// Jev clicks "Buy now" under the named backdrop of `covered.html` until the
/// stall rule ends the run, and the scripted fallback reads its brief and
/// gives up. The call's data, and the fallback's first message.
async fn covered_call() -> Option<(serde_json::Value, String)> {
    let harness = super::Harness::start().await?;
    let model = Arc::new(ScriptedFallback::from_json(
        json!([{"say": "BLOCKED: a popup covers the button"}]),
    ));
    let data = call_falling_back(
        &harness,
        &test_sessions(),
        "cover",
        json!({"goal": "Click Buy now.", "url": harness.site.url("covered.html")}),
        jev(json!([{"click": "Buy now", "repeat": 3}])),
        model.clone(),
        &ceilings(FallbackMode::Auto),
    )
    .await
    .unwrap();
    Some((data, model.opening()))
}

#[tokio::test]
async fn a_covered_click_names_its_cover_in_the_record_the_digest_and_the_fallback_brief() {
    let Some((mut data, opening)) = covered_call().await else {
        eprintln!("skipping: no Chrome binary found (set JEV_CHROME_BINARY)");
        return;
    };
    assert_eq!(data["stop_cause"], "covered", "{data:#}");
    let steps = data["actions"].as_array().unwrap();
    assert_eq!(steps.len(), 3, "{data:#}");
    for step in steps {
        assert_eq!(step["covered"], true, "{data:#}");
        assert_eq!(step["covered_by"], "Spring sale popup", "{data:#}");
    }

    let text = tool_text(&mut data);
    assert!(
        text.contains(r#"covered by "Spring sale popup"; nothing was done"#),
        "{text}"
    );

    assert!(
        opening.contains(r#"(covered by "Spring sale popup"; nothing was pressed)"#),
        "{opening}"
    );
}

/// What `act` says covers "Buy now" on `covered.html` once `setup` has
/// changed the page, which is then read afresh.
async fn cover_after(harness: &super::Harness, setup: &str) -> Option<String> {
    let mut page = harness.open("covered.html").await.unwrap();
    page.evaluate(setup).await.unwrap();
    let observation = page.observe().await.unwrap();
    let button = find(&observation, "click", "Buy now")
        .expect("the covered button is observed")
        .clone();
    let error = page
        .act(&button, &observation, None, Duration::from_millis(100))
        .await
        .unwrap_err();
    let covered = error.downcast_ref::<Covered>().expect("covered");
    // Whatever the page says, the message is Roder's own fixed text.
    assert_eq!(
        covered.to_string(),
        "Another element covers the target; nothing was clicked."
    );
    assert_eq!(page.evaluate("window.clicks ?? 0").await.unwrap(), json!(0));
    let cover = covered.cover().map(str::to_string);
    page.close().await.ok();
    cover
}

#[tokio::test]
async fn the_cover_is_named_by_its_label_then_its_text_then_its_tag() {
    let harness = harness_or_skip!();
    let overlay = "document.getElementById('overlay')";
    for (setup, named) in [
        // The fixture's own: a label.
        ("0", "Spring sale popup"),
        // A title says as much as a label does.
        (
            &format!("{overlay}.removeAttribute('aria-label'); {overlay}.title='Newsletter'")
                as &str,
            "Newsletter",
        ),
        // No label: the text it shows, on one line.
        (
            &format!(
                "{overlay}.removeAttribute('aria-label'); {overlay}.textContent='Subscribe  to our\\nnewsletter'"
            ),
            "Subscribe to our newsletter",
        ),
        // Nothing to read: its tag.
        (&format!("{overlay}.removeAttribute('aria-label')"), "div"),
        // The element hit has nothing; the one around it is the layer's name.
        (
            &format!("{overlay}.innerHTML='<span style=\"position:fixed;inset:0\"></span>'"),
            "Spring sale popup",
        ),
    ] {
        assert_eq!(
            cover_after(&harness, setup).await.as_deref(),
            Some(named),
            "{setup}"
        );
    }
}

/// A field laid over the button names itself by its label or placeholder,
/// never by what was typed in it, bar a button's own label.
#[tokio::test]
async fn a_covering_field_never_gives_its_value() {
    let harness = harness_or_skip!();
    let field = |attributes: &str| {
        format!(
            "document.getElementById('overlay').outerHTML='<input id=\"overlay\" {attributes} \
             style=\"position:fixed;inset:0;width:100%;height:100%\">'"
        )
    };
    for (setup, named) in [
        (field("type=password value=hunter2-secret"), "input"),
        (
            field("type=text value=hunter2-secret placeholder=Search"),
            "Search",
        ),
        (field("type=submit value=Accept"), "Accept"),
    ] {
        let cover = cover_after(&harness, &setup).await;
        assert_eq!(cover.as_deref(), Some(named), "{setup}");
        assert!(!cover.unwrap().contains("hunter2"));
    }
}

/// The element that takes the click may hold the target (a wrapper over a
/// button that ignores pointer events); its text is the target's own, so it
/// is not the cover's name.
#[tokio::test]
async fn a_wrapper_that_holds_the_target_is_not_named_by_the_targets_text() {
    let harness = harness_or_skip!();
    let setup = "document.getElementById('overlay').remove(); \
        document.getElementById('buy').style.pointerEvents='none'; \
        document.getElementById('buy').outerHTML='<section id=wrap>Order <button id=buy \
        style=\"pointer-events:none\" onclick=\"window.clicks=(window.clicks||0)+1\">Buy now</button></section>'";
    let cover = cover_after(&harness, setup).await;
    assert_eq!(cover.as_deref(), Some("section"), "named by its tag");
}

/// Open `anchored-form.html` with `query`, after checking that Login starts
/// under the band (or what replaced it), and read it.
async fn anchored(harness: &super::Harness, query: &str) -> (crate::page::Page, Value) {
    let mut page = harness
        .open(&format!("anchored-form.html?{query}"))
        .await
        .unwrap();
    let under = page
        .evaluate(
            "(r => document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)?.id)\
             (document.getElementById('go').getBoundingClientRect())",
        )
        .await
        .unwrap();
    assert_ne!(under, json!("go"), "{query}: Login must start covered");
    let observation = page.observe().await.unwrap();
    (page, observation)
}

/// What the page counted: Login pressed, the form submitted, the ad pressed.
async fn pressed(page: &mut crate::page::Page) -> Value {
    page.evaluate("[window.pressed ?? null, window.submitted ?? null, window.adClicks]")
        .await
        .unwrap()
}

/// expandtesting-login, as fastbrowse found it: an ad anchored to the
/// window's foot covered Login and the run ended stuck. A person scrolls the
/// button clear of the band; where the page cannot scroll, Login is pressed
/// from the keyboard. Nothing in the band is touched either way.
#[tokio::test]
async fn a_control_under_a_band_fixed_to_the_window_is_reached_without_touching_the_band() {
    let harness = harness_or_skip!();
    for (query, how) in [
        ("scroll=1", "scrolled it clear of \"Advertisement\""),
        (
            "scroll=0",
            "pressed it from the keyboard: \"Advertisement\" covers it",
        ),
    ] {
        let (mut page, observation) = anchored(&harness, query).await;
        let login = find(&observation, "click", "Login").unwrap().clone();
        let outcome = page
            .act(&login, &observation, None, Duration::from_millis(100))
            .await
            .unwrap_or_else(|error| panic!("{query}: {error:#}"));
        assert_eq!(outcome.uncovered.as_deref(), Some(how), "{query}");
        assert_eq!(pressed(&mut page).await, json!([1, 1, 0]), "{query}");
        page.close().await.ok();
    }
}

/// The same band over the password field of a page that cannot scroll: the
/// field is focused and typed into, and Enter submits it, from the keyboard.
#[tokio::test]
async fn a_field_under_a_band_is_filled_and_submitted_from_the_keyboard() {
    let harness = harness_or_skip!();
    let (mut page, observation) = anchored(&harness, "scroll=0").await;
    let password = find(&observation, "fill", "Password").unwrap().clone();
    let outcome = page
        .act(
            &password,
            &observation,
            Some("hunter2"),
            Duration::from_millis(100),
        )
        .await
        .unwrap();
    assert_eq!(outcome.refused, None);
    assert!(
        outcome
            .uncovered
            .as_deref()
            .is_some_and(|how| how.starts_with("typed into it from the keyboard")),
        "{outcome:?}"
    );
    let observation = page.observe().await.unwrap();
    let enter = find(&observation, "enter", "Password")
        .unwrap_or_else(|| panic!("{observation:#}"))
        .clone();
    page.act(&enter, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    let state = page
        .evaluate(
            "[document.querySelector('[name=pass]').value, window.submitted, window.adClicks]",
        )
        .await
        .unwrap();
    assert_eq!(state, json!(["hunter2", 1, 0]));
    page.close().await.ok();
}

/// Only a band floating over the page is routed around. A dialog asks its
/// question first, an open menu is the page's to close, and a cover over the
/// whole window shows something other than the control: none is pressed
/// through, by pointer or by keyboard (Enter in a covered field included).
#[tokio::test]
async fn nothing_is_pressed_through_a_cover_the_page_means_to_be_answered() {
    let harness = harness_or_skip!();
    for cover in ["dialog", "menu", "full"] {
        let (mut page, observation) = anchored(&harness, &format!("scroll=0&cover={cover}")).await;
        let login = find(&observation, "click", "Login").unwrap().clone();
        let clicked = page
            .act(&login, &observation, None, Duration::from_millis(100))
            .await;
        assert!(
            clicked.as_ref().is_err_and(|error| error.is::<Covered>()),
            "{cover}: {clicked:?}"
        );
        // Enter is offered only for the field last typed into: press it in
        // the observed password field as such an action would.
        let observation = page.observe().await.unwrap();
        let mut enter = find(&observation, "fill", "Password").unwrap().clone();
        enter["kind"] = json!("enter");
        let entered = page
            .act(&enter, &observation, None, Duration::from_millis(100))
            .await;
        assert!(
            entered.as_ref().is_err_and(|error| error.is::<Covered>()),
            "{cover}: {entered:?}"
        );
        assert_eq!(pressed(&mut page).await, json!([null, null, 0]), "{cover}");
        page.close().await.ok();
    }
}

/// A consent platform's bar fixed to the window's foot is shaped like a band
/// but asks a question first: it is left to banner refusal, and the target
/// under it is not scrolled clear of it or pressed from the keyboard.
#[tokio::test]
async fn a_consent_bar_is_no_band_to_get_around() {
    let harness = harness_or_skip!();
    let mut page = harness.open("cookie-cmp.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let show = find(&observation, "click", "Show more").unwrap().clone();
    let outcome = page
        .act(&show, &observation, None, Duration::from_millis(100))
        .await;
    assert!(
        outcome.as_ref().is_err_and(|error| error.is::<Covered>()),
        "{outcome:?}"
    );
    let state = page
        .evaluate("[window.more ?? false, window.clicks.length]")
        .await
        .unwrap();
    assert_eq!(state, json!([false, 0]));
    page.close().await.ok();
}

/// Focusing the control can open a menu over it while the control stays as
/// it was, and a focus handler can repurpose it: the cover and the control
/// are checked again after focusing, right before the key, and no key is
/// sent through the menu or to the repurposed control.
#[tokio::test]
async fn no_key_is_sent_through_what_focus_opened_or_to_a_control_focus_changed() {
    let harness = harness_or_skip!();
    let menu = "document.getElementById('go').addEventListener('focus', e => { \
        const r = e.target.getBoundingClientRect(), o = document.createElement('div'); \
        o.setAttribute('role', 'menu'); o.textContent = 'Opened on focus'; \
        o.style.cssText = 'position:absolute;z-index:99;background:#fff;left:' + (r.left - 10) + \
        'px;top:' + (r.top - 10) + 'px;width:' + (r.width + 20) + 'px;height:' + (r.height + 20) + 'px'; \
        document.body.append(o); }, {once: true}); true";
    let renamed = "document.getElementById('go').addEventListener('focus', e => { \
        e.target.textContent = 'Delete account'; }, {once: true}); true";
    for (setup, covered) in [(menu, true), (renamed, false)] {
        let (mut page, _) = anchored(&harness, "scroll=0").await;
        let counting =
            "window.keys = 0; document.addEventListener('keydown', () => window.keys++, true); ";
        let set = page.evaluate(&format!("{counting}{setup}")).await.unwrap();
        assert_eq!(set, json!(true), "{setup}");
        let observation = page.observe().await.unwrap();
        let login = find(&observation, "click", "Login").unwrap().clone();
        let error = page
            .act(&login, &observation, None, Duration::from_millis(100))
            .await
            .unwrap_err();
        match covered {
            true => assert!(error.is::<Covered>(), "{error:#}"),
            false => assert!(error.is::<StaleObservation>(), "{error:#}"),
        }
        let state = page
            .evaluate("[window.keys, window.pressed ?? null, window.adClicks]")
            .await
            .unwrap();
        assert_eq!(state, json!([0, null, 0]), "{setup}");
        page.close().await.ok();
    }
}

/// A members' offer closed only by "No thanks, take me to the shop": Jev
/// cannot tell that from the offer's other buttons, so the target stays
/// covered. The step names the offer (the dialog beside the bare backdrop
/// that was hit) and what it offers to press, in the record, the digest and
/// the fallback's brief, so the caller and the fallback can close it.
#[tokio::test]
async fn a_covered_step_names_what_the_cover_offers_to_press() {
    let Some(harness) = super::Harness::start().await else {
        eprintln!("skipping: no Chrome binary found (set JEV_CHROME_BINARY)");
        return;
    };
    let model = Arc::new(ScriptedFallback::from_json(
        json!([{"say": "BLOCKED: an offer covers the button"}]),
    ));
    let mut data = call_falling_back(
        &harness,
        &test_sessions(),
        "offer",
        json!({"goal": "Add the seeds to the basket.", "url": harness.site.url("odd-close.html")}),
        jev(json!([{"click": "Add to basket", "repeat": 3}])),
        model.clone(),
        &ceilings(FallbackMode::Auto),
    )
    .await
    .unwrap();
    let offers = json!(["Join for £5", "No thanks, take me to the shop"]);
    for step in data["actions"].as_array().unwrap() {
        assert_eq!(step["covered"], true, "{data:#}");
        assert_eq!(step["covered_by"], "Members-only offer", "{data:#}");
        assert_eq!(step["covered_controls"], offers, "{data:#}");
    }
    let named = "covered by \"Members-only offer\", which offers \"Join for £5\", \
                 \"No thanks, take me to the shop\"";
    let text = tool_text(&mut data);
    assert!(text.contains(named), "{text}");
    let opening = model.opening();
    assert!(opening.contains(named), "{opening}");
}

/// A field's own decoration drawn over its centre (a floating label's text,
/// an icon) is no cover: Enter in it is pressed as before. Only the centre
/// was looked at, so the label read as something else covering the field
/// and Enter was refused, though the field's other points are its own.
#[tokio::test]
async fn enter_is_pressed_in_a_field_whose_label_text_sits_over_its_centre() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    let set = page
        .evaluate(
            "(() => { const f = document.getElementById('chat'), r = f.getBoundingClientRect(); \
             const l = document.createElement('label'); l.htmlFor = 'chat'; \
             const s = document.createElement('span'); s.textContent = 'Say'; l.append(s); \
             l.style.cssText = 'position:absolute;z-index:5;background:#eee;font-size:10px;left:' + \
             (r.left + r.width / 2 - 12 + scrollX) + 'px;top:' + (r.top + r.height / 2 - 6 + scrollY) + \
             'px;width:24px;height:12px'; document.body.append(l); \
             const c = r.left + r.width / 2, m = r.top + r.height / 2; \
             return document.elementFromPoint(c, m) === s; })()",
        )
        .await
        .unwrap();
    assert_eq!(
        set,
        json!(true),
        "the label's text must sit over the centre"
    );
    let observation = page.observe().await.unwrap();
    let (_, typed) = super::act_on(&mut page, &observation, "fill", "Message", Some("hello"))
        .await
        .unwrap();
    let (outcome, sent) = super::act_on(&mut page, &typed, "enter", "Message", None)
        .await
        .unwrap_or_else(|error| panic!("{error:#}"));
    assert_eq!(outcome.uncovered, None);
    assert!(
        sent["text"].as_str().unwrap().contains("Sent: hello"),
        "{sent:#}"
    );
    page.close().await.ok();
}
