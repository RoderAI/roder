use roder_api::conversation::ToolResultRecord;
use roder_api::events::*;
use roder_api::tools::{ToolCall, ToolExecutionContext};
use time::OffsetDateTime;

use crate::runtime::Runtime;

impl Runtime {
    pub(crate) async fn route_tool_call(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        call: roder_api::inference::ToolCallCompleted,
    ) -> anyhow::Result<ToolResultRecord> {
        self.emit(RoderEvent::ToolCallRequested(ToolCallRequested {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: call.id.clone(),
            tool_name: call.name.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let Some(executor) = self.tool_registry.get(&call.name) else {
            let item = ToolResultRecord {
                id: call.id,
                name: Some(call.name),
                result: "tool not found".to_string(),
                is_error: true,
            };
            self.persist_turn_item(
                thread_id,
                turn_id,
                &roder_api::conversation::ConversationItem::ToolResult(item.clone()),
            )
            .await?;
            return Ok(item);
        };
        self.emit(RoderEvent::ToolCallStarted(ToolCallStarted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: call.id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let parsed_args = serde_json::from_str(&call.arguments)
            .unwrap_or_else(|_| serde_json::json!({ "raw": call.arguments }));
        let result = executor
            .execute(
                ToolExecutionContext {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                },
                ToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: parsed_args,
                    raw_arguments: call.arguments,
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                },
            )
            .await?;
        let item = ToolResultRecord {
            id: result.id.clone(),
            name: Some(result.name.clone()),
            result: result.text,
            is_error: result.is_error,
        };
        self.persist_turn_item(
            thread_id,
            turn_id,
            &roder_api::conversation::ConversationItem::ToolResult(item.clone()),
        )
        .await?;
        self.emit(RoderEvent::ToolCallCompleted(ToolCallCompleted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: result.id,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(item)
    }
}
