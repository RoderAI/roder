use super::*;

pub(super) fn responses_tools(
    request: &AgentInferenceRequest,
    profile: ResponsesProviderProfile,
) -> (Vec<Value>, ResponsesToolNameMap) {
    let mut tools = Vec::new();
    let mut used_tool_names = HashSet::new();
    let mut tool_name_map = ResponsesToolNameMap::default();
    match request.runtime.hosted_web_search.mode {
        HostedWebSearchMode::Disabled => {}
        HostedWebSearchMode::Cached => {
            let mut tool = json!({ "type": "web_search" });
            if profile != ResponsesProviderProfile::Xai {
                tool["external_web_access"] = json!(false);
            }
            tools.push(tool);
        }
        HostedWebSearchMode::Live => {
            let mut tool = json!({ "type": "web_search" });
            if profile != ResponsesProviderProfile::Xai {
                tool["external_web_access"] = json!(true);
            }
            tools.push(tool);
        }
    }
    for tool in &request.tools {
        if tool.name == roder_api::computer::COMPUTER_TOOL_NAME {
            tools.push(json!({"type":"computer"}));
            continue;
        }
        let tool = tool.normalized_for_model(roder_api::ToolSchemaPolicy::warning());
        let api_name = responses_tool_name(&tool.name, &mut used_tool_names);
        tool_name_map.register(&tool.name, &api_name);
        let mut entry = if freeform_custom_tool(&tool, request) {
            // Freeform/custom channel: the model emits the raw body (patch
            // text) as a string `input`; no JSON parameters schema is sent.
            json!({
                "type": "custom",
                "name": api_name,
                "description": "The `apply_patch` tool can be used to edit files. This is a FREEFORM tool, so do not wrap the patch in JSON.",
                "format": { "type": "grammar", "syntax": "lark", "definition": include_str!("../assets/apply_patch.lark") },
            })
        } else {
            json!({
                "type": "function",
                "name": api_name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        };
        if openai_provider_native_tool_search(request) {
            entry["defer_loading"] = json!(true);
        }
        tools.push(entry);
    }
    if openai_provider_native_tool_search(request)
        && request
            .tools
            .iter()
            .any(|tool| tool.name != roder_api::computer::COMPUTER_TOOL_NAME)
    {
        tools.push(json!({ "type": "tool_search", "execution": "client", "description": "Search deferred tool names and descriptions and load their full definitions.", "parameters": { "type": "object", "properties": {"query": {"type":"string"}, "limit": {"type":"integer", "minimum":0}}, "required":["query"], "additionalProperties":false } }));
    }
    (tools, tool_name_map)
}

pub(super) fn openai_provider_native_tool_search(request: &AgentInferenceRequest) -> bool {
    request.runtime.tool_search.is_provider_native_requested()
        && request.model.provider == PROVIDER_OPENAI
        && openai_model_supports_tool_search(&request.model.model)
}

/**
 * Whether an OpenAI model id is known to support Responses `tool_search`.
 * Public so offline eval fixtures exercise the same support gating as the
 * live request mapping.
 */
pub fn openai_model_supports_tool_search(model: &str) -> bool {
    model.starts_with("gpt-5.4")
        || model.starts_with("gpt-5.5")
        || model.starts_with("gpt-5.6")
        || is_gpt_6_family(model)
}

/// GPT-6 family ids: `gpt-6`, `gpt-6-*` releases, and point releases such as `gpt-6.1-sol`.
fn is_gpt_6_family(model: &str) -> bool {
    model == "gpt-6" || model.starts_with("gpt-6-") || model.starts_with("gpt-6.")
}

/// GPT-5 and GPT-6 coding models use Codex's grammar-constrained custom tool.
pub fn openai_model_supports_freeform_apply_patch(model: &str) -> bool {
    model.starts_with("gpt-5")
        || model.starts_with("gpt-6")
        || model.starts_with("gpt-daybreak-")
        || model == "codex-auto-review"
}

pub(super) fn freeform_custom_tool(
    tool: &roder_api::tools::ToolSpec,
    request: &AgentInferenceRequest,
) -> bool {
    tool.freeform_input_field().is_some()
        && matches!(
            request.model.provider.as_str(),
            PROVIDER_OPENAI | PROVIDER_CODEX
        )
        && openai_model_supports_freeform_apply_patch(&request.model.model)
}
