use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use serde_json::json;

pub use roder_api::tools::*;

#[derive(Debug, Default)]
pub struct EchoToolContributor;

impl ToolContributor for EchoToolContributor {
    fn id(&self) -> ToolProviderId {
        "builtin-echo".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(EchoTool));
        Ok(())
    }
}

#[derive(Debug)]
pub struct EchoTool;

#[async_trait::async_trait]
impl ToolExecutor for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".to_string(),
            description: "Returns the provided text argument unchanged.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Text to return."
                    }
                },
                "required": ["text"]
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let text = call
            .arguments
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&call.raw_arguments)
            .to_string();
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: text.clone(),
            data: json!({ "text": text }),
            is_error: false,
        })
    }
}

pub fn echo_tool_contributor() -> Arc<dyn ToolContributor> {
    Arc::new(EchoToolContributor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_contributor_registers_echo_spec() {
        let mut registry = ToolRegistry::default();
        EchoToolContributor.contribute(&mut registry).unwrap();

        let specs = registry.specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "echo");
        assert!(registry.get("echo").is_some());
    }

    #[tokio::test]
    async fn echo_tool_returns_text_argument() {
        let tool = EchoTool;
        let result = tool
            .execute(
                ToolExecutionContext {
                    thread_id: "thread-a".to_string(),
                    turn_id: "turn-a".to_string(),
                },
                ToolCall {
                    id: "call-a".to_string(),
                    name: "echo".to_string(),
                    arguments: json!({ "text": "hello harness" }),
                    raw_arguments: "{}".to_string(),
                    thread_id: "thread-a".to_string(),
                    turn_id: "turn-a".to_string(),
                },
            )
            .await
            .unwrap();

        assert_eq!(result.text, "hello harness");
        assert_eq!(result.data, json!({ "text": "hello harness" }));
        assert!(!result.is_error);
    }
}
