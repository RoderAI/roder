use roder_api::backend::{BackendHistoryItem, BackendHistoryTurn, BackendThreadSnapshot};
use serde_json::Value;

use crate::mapping;

fn turn(snapshot: &mut BackendThreadSnapshot) -> &mut BackendHistoryTurn {
    if snapshot.turns.is_empty() {
        new_turn(snapshot);
    }
    snapshot.turns.last_mut().expect("history turn was created")
}

fn new_turn(snapshot: &mut BackendThreadSnapshot) {
    snapshot.turns.push(BackendHistoryTurn {
        id: format!("antigravity-replay-{}", snapshot.turns.len() + 1),
        status: "completed".into(),
        started_at: None,
        completed_at: None,
        items: Vec::new(),
    });
}

/// Record ACP replay and live chunks independently of event delivery.
pub(crate) fn record(snapshot: &mut BackendThreadSnapshot, update: &Value) {
    match update["sessionUpdate"].as_str().unwrap_or("") {
        "user_message_chunk" => {
            let text = mapping::text(&update["content"]);
            if text.is_empty() {
                return;
            }
            // send() already records the complete user message for a live turn.
            if snapshot.turns.last().is_some_and(|turn| {
                turn.status == "in_progress"
                    && turn
                        .items
                        .iter()
                        .any(|item| matches!(item, BackendHistoryItem::User { .. }))
            }) {
                return;
            }
            if snapshot.turns.last().is_some_and(|turn| {
                !turn.items.is_empty()
                    && !matches!(turn.items.last(), Some(BackendHistoryItem::User { .. }))
            }) {
                new_turn(snapshot);
            }
            if snapshot.preview.is_empty() {
                snapshot.preview = text.chars().take(160).collect();
            }
            let turn = turn(snapshot);
            if let Some(BackendHistoryItem::User { text: existing, .. }) = turn.items.last_mut() {
                existing.push_str(&text);
            } else {
                turn.items.push(BackendHistoryItem::User {
                    id: format!("{}-user-{}", turn.id, turn.items.len()),
                    text,
                });
            }
        }
        "agent_message_chunk" => {
            let text = mapping::text(&update["content"]);
            if text.is_empty() {
                return;
            }
            let turn = turn(snapshot);
            if let Some(BackendHistoryItem::Agent { text: existing, .. }) = turn.items.last_mut() {
                existing.push_str(&text);
            } else {
                turn.items.push(BackendHistoryItem::Agent {
                    id: format!("{}-agent-{}", turn.id, turn.items.len()),
                    text,
                });
            }
        }
        "tool_call" | "tool_call_update" => {
            let Some(id) = update["toolCallId"].as_str().filter(|id| !id.is_empty()) else {
                return;
            };
            let name = update["title"].as_str();
            let status = update["status"].as_str();
            // ACP tool updates replace supplied fields; absent fields retain prior values.
            let output = update.get("content").map(mapping::text);
            let existing = snapshot.turns.iter_mut().rev().find_map(|turn| {
                turn.items.iter_mut().find(|item| {
                    matches!(item, BackendHistoryItem::Tool { id: existing, .. } if existing == id)
                })
            });
            if let Some(BackendHistoryItem::Tool {
                name: existing_name,
                output: existing_output,
                status: existing_status,
                ..
            }) = existing
            {
                if let Some(name) = name {
                    *existing_name = name.into();
                }
                if let Some(status) = status {
                    *existing_status = status.into();
                }
                if let Some(output) = output {
                    *existing_output = Some(output);
                }
            } else {
                turn(snapshot).items.push(BackendHistoryItem::Tool {
                    id: id.into(),
                    name: name.unwrap_or("Antigravity tool").into(),
                    output,
                    status: status.unwrap_or("pending").into(),
                });
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chunk(snapshot: &mut BackendThreadSnapshot, role: &str, text: &str) {
        record(
            snapshot,
            &json!({
                "sessionUpdate": role,
                "content": {"type":"text", "text":text}
            }),
        );
    }

    #[test]
    fn replay_groups_chunks_into_multiple_turns() {
        let mut snapshot = BackendThreadSnapshot::default();
        chunk(&mut snapshot, "user_message_chunk", "first ");
        chunk(&mut snapshot, "user_message_chunk", "question");
        chunk(&mut snapshot, "agent_message_chunk", "first ");
        chunk(&mut snapshot, "agent_message_chunk", "answer");
        chunk(&mut snapshot, "user_message_chunk", "second question");
        chunk(&mut snapshot, "agent_message_chunk", "second answer");
        assert_eq!(snapshot.turns.len(), 2);
        assert_ne!(snapshot.turns[0].id, snapshot.turns[1].id);
        assert!(matches!(&snapshot.turns[0].items[..], [
            BackendHistoryItem::User { text: user, .. },
            BackendHistoryItem::Agent { text: agent, .. }
        ] if user == "first question" && agent == "first answer"));
        assert!(matches!(&snapshot.turns[1].items[..], [
            BackendHistoryItem::User { text: user, .. },
            BackendHistoryItem::Agent { text: agent, .. }
        ] if user == "second question" && agent == "second answer"));
    }

    #[test]
    fn partial_tool_updates_preserve_identity_and_replace_supplied_content() {
        let mut snapshot = BackendThreadSnapshot::default();
        chunk(&mut snapshot, "user_message_chunk", "run test");
        record(
            &mut snapshot,
            &json!({
                "sessionUpdate":"tool_call", "toolCallId":"test-1",
                "title":"Run tests", "status":"in_progress",
                "content":[{"type":"content","content":{"type":"text","text":"starting"}}]
            }),
        );
        record(
            &mut snapshot,
            &json!({
                "sessionUpdate":"tool_call_update", "toolCallId":"test-1",
                "content":[{"type":"content","content":{"type":"text","text":"passed"}}]
            }),
        );
        record(
            &mut snapshot,
            &json!({
                "sessionUpdate":"tool_call_update", "toolCallId":"test-1", "status":"completed"
            }),
        );
        chunk(&mut snapshot, "agent_message_chunk", "done");
        assert_eq!(snapshot.turns[0].items.len(), 3);
        assert!(matches!(&snapshot.turns[0].items[1],
            BackendHistoryItem::Tool { id, name, output: Some(output), status }
            if id == "test-1" && name == "Run tests" && output == "passed" && status == "completed"));
        chunk(&mut snapshot, "user_message_chunk", "next");
        record(
            &mut snapshot,
            &json!({
                "sessionUpdate":"tool_call_update", "toolCallId":"test-1", "status":"failed"
            }),
        );
        assert_eq!(snapshot.turns[1].items.len(), 1);
        assert!(matches!(&snapshot.turns[0].items[1],
            BackendHistoryItem::Tool { status, .. } if status == "failed"));
    }

    #[test]
    fn live_turn_keeps_user_message_and_records_assistant_once() {
        let mut snapshot = BackendThreadSnapshot::default();
        snapshot.turns.push(BackendHistoryTurn {
            id: "rpc-5".into(),
            status: "in_progress".into(),
            started_at: None,
            completed_at: None,
            items: vec![BackendHistoryItem::User {
                id: "rpc-5-user".into(),
                text: "hello".into(),
            }],
        });
        chunk(&mut snapshot, "user_message_chunk", "hello");
        chunk(&mut snapshot, "agent_message_chunk", "hello ");
        chunk(&mut snapshot, "agent_message_chunk", "back");
        assert_eq!(snapshot.turns.len(), 1);
        assert!(matches!(&snapshot.turns[0].items[..], [
            BackendHistoryItem::User { text: user, .. },
            BackendHistoryItem::Agent { text: agent, .. }
        ] if user == "hello" && agent == "hello back"));
    }

    #[test]
    fn replay_without_user_or_initial_tool_notification_is_retained() {
        let mut snapshot = BackendThreadSnapshot::default();
        record(
            &mut snapshot,
            &json!({
                "sessionUpdate":"tool_call_update", "toolCallId":"late-tool",
                "title":"Read file", "status":"completed"
            }),
        );
        chunk(&mut snapshot, "agent_message_chunk", "summary");
        record(&mut snapshot, &json!({"sessionUpdate":"tool_call_update"}));
        assert_eq!(snapshot.turns.len(), 1);
        assert_eq!(snapshot.turns[0].items.len(), 2);
        assert!(matches!(&snapshot.turns[0].items[0],
            BackendHistoryItem::Tool { id, .. } if id == "late-tool"));
    }
}
