//! A batch gives up its retries after two 5xx answers in a row, so its
//! parts can be asked one at a time within what is left.

use serde_json::json;

use super::tests::{MockServer, Reply, fast_policy};
use super::*;

fn splittable(spent: usize) -> Schedule {
    Schedule { spent, split: true }
}

#[tokio::test]
async fn two_server_errors_in_a_row_split_a_batch() {
    let server = MockServer::start(vec![Reply::status(503), Reply::status(500)]).await;
    let poster = JsonPoster::new(fast_policy());

    let posted = poster
        .post_scheduled(&server.url, "k", &[], &json!({}), splittable(0))
        .await;

    assert!(
        matches!(
            posted,
            Err(Posted::Split {
                spent: 2,
                failure: PostFailure::Status(500, None)
            })
        ),
        "{posted:?}"
    );
    assert_eq!(server.hits(), 2);
}

#[tokio::test]
async fn a_server_error_between_others_does_not_split() {
    let server = MockServer::start(vec![
        Reply::status(503),
        Reply::status(429),
        Reply::status(503),
        Reply::ok(r#"{"ok":1}"#),
    ])
    .await;
    let poster = JsonPoster::new(fast_policy());

    let posted = poster
        .post_scheduled(&server.url, "k", &[], &json!({}), splittable(0))
        .await;

    assert_eq!(posted.unwrap(), (json!({"ok": 1}), 4));
}

#[tokio::test]
async fn without_split_server_errors_use_every_retry() {
    let server = MockServer::start(vec![Reply::status(503)]).await;
    let poster = JsonPoster::new(fast_policy());

    let posted = poster.post(&server.url, "k", &json!({})).await;

    assert_eq!(posted, Err(PostFailure::Status(503, None)));
    assert_eq!(server.hits(), fast_policy().delays.len() + 1);
}

#[tokio::test]
async fn a_part_starts_with_only_the_retries_left() {
    let server = MockServer::start(vec![Reply::status(503)]).await;
    let poster = JsonPoster::new(fast_policy());
    let retries = fast_policy().delays.len();

    let posted = poster
        .post_scheduled(
            &server.url,
            "k",
            &[],
            &json!({}),
            Schedule {
                spent: retries - 1,
                split: false,
            },
        )
        .await;

    assert!(matches!(posted, Err(Posted::Failed(_))));
    // Its own first request, and the one retry the batch left it.
    assert_eq!(server.hits(), 2);
}
