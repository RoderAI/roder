use super::*;
use roder_api::events::TurnInterrupted;
use roder_api::inference::TokenUsage;
use serde_json::json;

#[test]
fn interrupted_completion_notification_preserves_usage_and_status() {
    let usage = TokenUsage {
        prompt_tokens: 100,
        completion_tokens: 20,
        total_tokens: 120,
        cached_prompt_tokens: 30,
        cache_creation_prompt_tokens: 10,
        service_tier: Some("priority".into()),
        ..TokenUsage::default()
    };
    let interruption: TurnInterrupted = serde_json::from_value(json!({
        "thread_id": "thread",
        "turn_id": "interrupted",
        "usage": usage,
        "timestamp": "1970-01-01T00:00:01Z"
    }))
    .unwrap();
    let notifications =
        protocol_notifications_for_event(&RoderEvent::TurnInterrupted(interruption));

    assert_eq!(notifications.len(), 2);
    assert_eq!(notifications[0].method, "turn/completed");
    assert_eq!(notifications[0].params["threadId"], "thread");
    let turn = &notifications[0].params["turn"];
    assert_eq!(turn["status"], "interrupted");
    assert!(turn.get("error").is_none_or(serde_json::Value::is_null));
    assert_eq!(turn["completedAt"], 1);
    assert_eq!(turn["usage"], serde_json::to_value(usage).unwrap());
    assert_eq!(notifications[1].method, "thread/status/changed");
    assert_eq!(notifications[1].params["status"]["type"], "idle");
}
