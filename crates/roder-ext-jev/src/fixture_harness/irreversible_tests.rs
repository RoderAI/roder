//! The irreversible-action gate on real pages: what the snapshot says about
//! a control (where a link goes, whether it saves a file, the form a submit
//! sends) and the questions the gate builds from it.

use serde_json::Value;

use crate::decide::request_body;
use crate::irreversible::{self, Answers, DOWNLOADS_KEY, Verdict};

fn action<'a>(observation: &'a Value, kind: &str, label: &str) -> &'a Value {
    observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["kind"] == kind && action["label"] == label)
        .unwrap_or_else(|| panic!("no {kind} {label:?} in {}", observation["actions"]))
}

/// The gate's questions for this observation, by key.
fn questions(observation: &Value, goal: &str) -> serde_json::Map<String, Value> {
    let (mut body, space, _) = request_body(
        observation,
        goal,
        &[],
        "jev-latest",
        chrono::NaiveDate::default(),
    );
    irreversible::add_questions(&mut body, &space, goal);
    body["questions"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(key, _)| irreversible::is_gate_key(key))
        .map(|(key, question)| (key.clone(), question.clone()))
        .collect()
}

/// The text of the question about the element labelled `label`.
fn asked_about<'a>(questions: &'a serde_json::Map<String, Value>, label: &str) -> Option<&'a str> {
    let quoted = format!("labelled {label:?}");
    questions
        .values()
        .filter_map(|question| question["instructions"].as_str())
        .find(|text| text.contains(&quoted))
}

#[tokio::test]
async fn the_snapshot_says_where_a_link_goes_and_what_a_submit_sends() {
    let harness = harness_or_skip!();
    let mut page = harness.open("unsubscribe-link.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let stop = action(&observation, "click", "Stop these emails");
    assert_eq!(
        stop["href"],
        "/pages/unsubscribe.html?list=garden&token=k3x9"
    );
    assert!(stop.get("download").is_none());
    assert!(stop.get("submits").is_none());
    // A jump within the page goes nowhere new.
    assert_eq!(
        action(&observation, "click", "Read the Garden club posts")["href"],
        "#posts"
    );
    page.close().await.unwrap();

    let mut page = harness.open("change-password.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let save = action(&observation, "click", "Save");
    assert_eq!(save["submits"]["method"], "post");
    assert_eq!(save["submits"]["action"], "/submit/password");
    assert!(
        save["submits"]["text"]
            .as_str()
            .unwrap()
            .contains("Confirm new password"),
        "{save}"
    );
    // A field's Enter sends the form its submit button would.
    let field = action(&observation, "fill", "New password");
    assert_eq!(field["submits"], save["submits"]);
    // Fields named like the form's own properties shadow them; the
    // observation still reads the form, and still succeeds.
    page.evaluate(
        "document.querySelector('form').insertAdjacentHTML('afterbegin', \
         '<select id=\"method\"><option>Email</option></select><input type=\"hidden\" \
         name=\"action\">'); true",
    )
    .await
    .unwrap();
    let observation = page.observe().await.unwrap();
    let shadowed = action(&observation, "click", "Save");
    assert_eq!(shadowed["submits"]["method"], "post");
    assert_eq!(shadowed["submits"]["action"], "/submit/password");
    page.close().await.unwrap();

    let mut page = harness.open("download-zip.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let archive = action(&observation, "click", "Get all release notes");
    assert_eq!(archive["href"], "/pages/files/release-notes-2.1.zip");
    assert!(archive.get("download").is_none());
    page.evaluate("document.querySelector('a').setAttribute('download', ''); true")
        .await
        .unwrap();
    let observation = page.observe().await.unwrap();
    assert_eq!(
        action(&observation, "click", "Get all release notes")["download"],
        true
    );
    page.close().await.unwrap();
}

#[tokio::test]
async fn the_gate_asks_about_what_the_page_says_commits_and_quotes_it() {
    let harness = harness_or_skip!();
    // A one-click unsubscribe, by its address.
    let mut page = harness.open("unsubscribe-link.html").await.unwrap();
    let asked = questions(&page.observe().await.unwrap(), "Unsubscribe me.");
    let stop = asked_about(&asked, "Stop these emails").expect("asked about the unsubscribe");
    assert!(
        stop.contains("It is a link to \"/pages/unsubscribe.html?list=garden&token=k3x9\"."),
        "{stop}"
    );
    assert!(asked_about(&asked, "Read the Garden club posts").is_none());
    page.close().await.unwrap();

    // A dialog's "Yes, I'm sure", by the heading it sits under.
    let mut page = harness.open("confirm-delete.html").await.unwrap();
    page.evaluate("document.getElementById('confirm').hidden = false; true")
        .await
        .unwrap();
    let asked = questions(&page.observe().await.unwrap(), "Delete the Atlas project.");
    let sure = asked_about(&asked, "Yes, I'm sure").expect("asked about the confirmation");
    assert!(
        sure.contains("It sits under \"Delete the Atlas project?\" on the page."),
        "{sure}"
    );
    assert!(
        sure.contains("It submits a form (POST \"/submit/delete-project\") that reads"),
        "{sure}"
    );
    // Its sibling is asked about by the same heading, and its question says
    // it sends no form.
    let keep = asked_about(&asked, "Keep it").expect("asked about its sibling");
    assert!(!keep.contains("submits a form"), "{keep}");
    page.close().await.unwrap();

    // "Bestellen" is an order.
    let mut page = harness.open("bestellen.html").await.unwrap();
    let asked = questions(&page.observe().await.unwrap(), "Bestelle die Lampe.");
    assert!(asked_about(&asked, "Bestellen").is_some(), "{asked:?}");
    assert!(asked_about(&asked, "Zurück zum Warenkorb").is_none());
    page.close().await.unwrap();

    // Playback under "Publish your podcast" is never asked about; the
    // publish is.
    let mut page = harness.open("media-player.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    assert_eq!(
        action(&observation, "click", "Pause")["section"],
        "Publish your podcast"
    );
    let asked = questions(&observation, "Pause the preview.");
    for label in ["Play", "Pause", "Mute"] {
        assert!(asked_about(&asked, label).is_none(), "{label}");
        let control = action(&observation, "click", label);
        let verdict = irreversible::verdict(
            control,
            observation["actions"].as_array().unwrap(),
            Answers::default(),
            false,
            1.0,
        );
        assert_eq!(verdict, Verdict::Dispatch, "{label}");
    }
    assert!(asked_about(&asked, "Publish episode").is_some());
    page.close().await.unwrap();
}

#[tokio::test]
async fn a_new_password_and_a_forbidden_download_stop_on_the_pages_evidence() {
    let harness = harness_or_skip!();
    let mut page = harness.open("change-password.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let offered = observation["actions"].as_array().unwrap();
    // Not asked about: the model saying it is harmless changes nothing.
    let asked = questions(&observation, "Change my password.");
    assert!(asked_about(&asked, "Save").is_none(), "{asked:?}");
    let save = action(&observation, "click", "Save");
    let answers = Answers {
        irreversible: Some(0.01),
        downloads_forbidden: None,
    };
    let Verdict::Confirm(reason) = irreversible::verdict(save, offered, answers, false, 1.0) else {
        panic!("a password change dispatched");
    };
    assert!(reason.contains("password or credentials"), "{reason}");
    page.close().await.unwrap();

    let mut page = harness.open("download-zip.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let goal = "What is new in 2.1? This is read-only.";
    let asked = questions(&observation, goal);
    assert!(
        asked[DOWNLOADS_KEY]["instructions"]
            .as_str()
            .unwrap()
            .contains(goal)
    );
    // The link's label names nothing, so only the downloads answer counts.
    assert!(asked_about(&asked, "Get all release notes").is_none());
    let archive = action(&observation, "click", "Get all release notes");
    let offered = observation["actions"].as_array().unwrap();
    let verdict = |downloads_forbidden| {
        irreversible::verdict(
            archive,
            offered,
            Answers {
                irreversible: None,
                downloads_forbidden,
            },
            true,
            1.0,
        )
    };
    assert!(matches!(verdict(Some(0.9)), Verdict::Confirm(_)));
    assert_eq!(verdict(Some(0.1)), Verdict::Dispatch);
    page.close().await.unwrap();
}
