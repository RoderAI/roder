use crate::Runtime;
use roder_api::{
    inference::ToolCallCompleted,
    thread_events::ThreadEventSubscribeRequest,
    tools::{ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const NAMES: [&str; 4] = [
    "subscribe_event",
    "list_event_subscriptions",
    "cancel_event_subscription",
    "publish_event",
];
pub(crate) fn is_event_tool(name: &str) -> bool {
    NAMES.contains(&name)
}
pub(crate) fn contribute_tools(registry: &mut ToolRegistry) -> anyhow::Result<()> {
    for name in NAMES {
        registry.register(Arc::new(EventTool(name)))?;
    }
    Ok(())
}
struct EventTool(&'static str);
#[async_trait::async_trait]
impl ToolExecutor for EventTool {
    fn spec(&self) -> ToolSpec {
        let (description, properties, required) = match self.0 {
            "subscribe_event" => (
                "Subscribe this thread to a runtime-local topic, optionally with a one-shot delay or recurring timer. Use only for requested follow-ups such as CI babysitting. Ticks wait until the thread is idle and preserve its context. Subscriptions expire within 24 hours and do not survive harness exit; use durable automations for unattended schedules. Cancel when finished.",
                json!({"topic":{"type":"string"},"prompt":{"type":"string"},"delaySeconds":{"type":["integer","null"],"minimum":1,"maximum":86400,"description":"First timer delay in seconds. Set intervalSeconds to null for a one-shot. Null with no interval means publish-only."},"intervalSeconds":{"type":["integer","null"],"minimum":1,"maximum":86400,"description":"Recurring timer cadence. Set to null or omit for a one-shot delay or a publish-only subscription. Do not copy the delay here for one-shot reminders."},"maxDeliveries":{"type":"integer","minimum":1,"maximum":1000,"default":100},"ttlSeconds":{"type":"integer","minimum":1,"maximum":86400,"default":86400}}),
                vec!["topic", "prompt"],
            ),
            "list_event_subscriptions" => (
                "List this thread's live topic and timer subscriptions, including next tick time, last admitted delivery, pending delivery, and last errors.",
                json!({}),
                vec![],
            ),
            "cancel_event_subscription" => (
                "Cancel a subscription belonging to this thread, discarding pending ticks.",
                json!({"subscriptionId":{"type":"string"}}),
                vec!["subscriptionId"],
            ),
            _ => (
                "Publish a runtime-local topic event to threads that explicitly subscribed. Coalesces to one pending event per subscription; returns acceptance count, not delivery confirmation.",
                json!({"topic":{"type":"string"},"message":{"type":"string"}}),
                vec!["topic", "message"],
            ),
        };
        ToolSpec {
            name: self.0.to_string(),
            description: description.to_string(),
            parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
        }
    }
    async fn execute(
        &self,
        _: ToolExecutionContext,
        _: roder_api::tools::ToolCall,
    ) -> anyhow::Result<ToolResult> {
        anyhow::bail!("thread event tools require the Roder runtime")
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cancel {
    subscription_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    topic: String,
    message: String,
}

impl Runtime {
    pub(crate) async fn execute_event_tool(
        self: &Arc<Self>,
        thread_id: &str,
        call: &ToolCallCompleted,
        arguments: Value,
    ) -> ToolResult {
        let outcome: anyhow::Result<Value> = async {
            Ok(match call.name.as_str() {
                "subscribe_event" => {
                    let request: ThreadEventSubscribeRequest = serde_json::from_value(arguments)?;
                    json!({"subscription":self.subscribe_thread_event(thread_id,request).await?})
                },
                "list_event_subscriptions" => json!({"subscriptions":self.list_thread_event_subscriptions(thread_id).await}),
                "cancel_event_subscription" => {
                    let args: Cancel = serde_json::from_value(arguments)?;
                    json!({"cancelled":self.cancel_thread_event_subscription(thread_id,&args.subscription_id).await})
                },
                "publish_event" => {
                    let args: Publish = serde_json::from_value(arguments)?;
                    json!({"queuedSubscriptions":self.publish_thread_event(&args.topic,&args.message).await?})
                },
                _ => anyhow::bail!("unknown thread event tool"),
            })
        }.await;
        let (data, is_error) = match outcome {
            Ok(data) => (data, false),
            Err(error) => (json!({"error":error.to_string()}), true),
        };
        ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            text: data.to_string(),
            data,
            is_error,
        }
    }
}
