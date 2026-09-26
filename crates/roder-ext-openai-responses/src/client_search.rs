use super::*;

pub(super) struct SearchExecution {
    pub events: Vec<InferenceEvent>,
    pub history: Vec<Value>,
}

pub(super) fn execute_client_searches(
    ctx: &mut ClientToolSearchContext,
    state: &mut ResponsesStreamState,
    pending: Vec<Value>,
) -> anyhow::Result<SearchExecution> {
    let input = ctx
        .body
        .get_mut("input")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            ProviderFailure::new(
                ProviderFailureKind::Protocol,
                "Responses tool-search continuation requires input items",
            )
        })?;
    input.extend(state.response_output.clone());
    let mut execution = SearchExecution {
        events: Vec::new(),
        history: state.response_output.clone(),
    };
    for item in pending {
        let call_id = item
            .get("call_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                ProviderFailure::new(
                    ProviderFailureKind::Protocol,
                    "client tool_search_call requires call_id",
                )
            })?;
        let args: ClientSearchArguments = serde_json::from_value(
            item.get("arguments").cloned().unwrap_or(Value::Null),
        )
        .map_err(|error| {
            ProviderFailure::new(
                ProviderFailureKind::Protocol,
                format!("invalid client tool-search arguments: {error}"),
            )
        })?;
        let hits = ctx.catalog.search(&args.query, args.limit.unwrap_or(10));
        let selected: Vec<_> = hits.iter().map(|hit| json!(hit.name)).collect();
        let tools: Vec<_> = hits
            .iter()
            .map(|hit| {
                ctx.definitions.get(&hit.name).cloned().ok_or_else(|| {
                    ProviderFailure::new(
                        ProviderFailureKind::Protocol,
                        format!("missing loadable definition for {}", hit.name),
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        execution.events.extend(emit_hosted_tool_completed_events(
            HostedToolCallCompleted {
                id: item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or(call_id)
                    .into(),
                name: "tool_search".into(),
                arguments:
                    json!({"query":args.query,"selected_tools":selected,"executor":"client"})
                        .to_string(),
            },
            state,
        ));
        let output = json!({"type":"tool_search_output","call_id":call_id,"status":"completed","execution":"client","tools":tools});
        input.push(output.clone());
        execution
            .events
            .push(InferenceEvent::OutputItemCompleted(output.clone()));
        execution.history.push(output);
    }
    Ok(execution)
}

/**
 * Connection/request context for the client-executed `tool_search_call` →
 * `tool_search_output` flow (roadmap phase 79): when the model emits a
 * `tool_search_call` item without provider-side results, the client runs
 * the search against the runtime catalog adapter and continues the same
 * turn with a follow-up request carrying the `tool_search_output` item.
 */
#[derive(Clone)]
pub(super) struct ClientToolSearchContext {
    pub(super) base_url: String,
    pub(super) api_key: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) grok_conversation_id: Option<String>,
    pub(super) body: Value,
    pub(super) policy: Option<ReliabilityRequestPolicy>,
    pub(super) definitions: HashMap<String, Value>,
    pub(super) catalog: roder_api::tool_search_catalog::ToolSearchCatalog,
}

#[derive(Deserialize)]
struct ClientSearchArguments {
    query: String,
    limit: Option<usize>,
}

pub(super) fn client_search_definitions(
    body: &Value,
    map: &ResponsesToolNameMap,
) -> HashMap<String, Value> {
    body.get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|definition| {
            let api_name = definition.get("name").and_then(Value::as_str)?;
            let canonical = map
                .api_name_to_tool_name
                .get(api_name)
                .map(String::as_str)
                .unwrap_or(api_name);
            let mut definition = definition.clone();
            definition.as_object_mut()?.remove("defer_loading");
            Some((canonical.to_string(), definition))
        })
        .collect()
}
