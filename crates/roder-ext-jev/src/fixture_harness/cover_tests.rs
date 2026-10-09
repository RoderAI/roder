//! What covers a target is named: in the step's record, in the digest the
//! caller reads, and in the fallback's brief. Without the name every refusal
//! read alike ("another element covers the target"), and a caller or a
//! fallback could not tell a sale popup it should close from a menu it
//! opened itself.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::fallback_script::ScriptedFallback;
use super::fallback_tests::{ceilings, jev};
use super::scripted::find;
use super::sessions::{call_falling_back, test_sessions};
use crate::engine::Covered;
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
