use super::*;
use serde_json::json;

fn completion(turn: serde_json::Value) -> JsonRpcNotification {
    JsonRpcNotification {
        jsonrpc: "2.0".into(),
        method: "turn/completed".into(),
        params: json!({"threadId": "thread-1", "turn": turn}),
    }
}

#[test]
fn failed_turn_without_message_still_returns_error() {
    for error in [serde_json::Value::Null, json!({}), json!({"message": ""})] {
        let error = turn_completed_stop_reason(&completion(json!({
            "id": "turn-1", "status": "failed", "error": error,
            "finishReason": "end_turn",
        })))
        .unwrap_err();
        assert_eq!(error.code, -32603);
        assert_eq!(error.message, "Roder turn failed");
        assert_eq!(error.data.unwrap()["turnId"], "turn-1");
    }
}

#[test]
fn successful_and_interrupted_turns_keep_their_stop_reasons() {
    for (reason, expected) in [
        ("end_turn", acp::StopReason::EndTurn),
        ("length", acp::StopReason::MaxTokens),
        ("max_tokens", acp::StopReason::MaxTokens),
        ("max_turn_requests", acp::StopReason::MaxTurnRequests),
        ("refusal", acp::StopReason::Refusal),
        ("cancelled", acp::StopReason::Cancelled),
    ] {
        assert_eq!(
            turn_completed_stop_reason(&completion(json!({
                "status": "completed", "finishReason": reason,
            })))
            .unwrap(),
            expected,
        );
    }
    assert_eq!(
        turn_completed_stop_reason(&completion(json!({
            "status": "interrupted", "finishReason": "end_turn",
        })))
        .unwrap(),
        acp::StopReason::Cancelled,
    );
}
