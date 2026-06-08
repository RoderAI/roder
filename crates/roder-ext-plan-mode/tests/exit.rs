use roder_api::extension::{ExtensionRegistryBuilder, ProvidedService, RoderExtension};
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry};
use roder_ext_plan_mode::{ExitPlanModeTool, PlanModeExtension};
use serde_json::json;

#[test]
fn extension_advertises_policy_contributor() {
    let manifest = PlanModeExtension::new(PolicyMode::Default).manifest();

    assert!(
        manifest
            .provides
            .contains(&ProvidedService::PolicyContributor("plan-mode".to_string()))
    );
    assert!(manifest.provides.contains(&ProvidedService::ToolProvider(
        "plan-mode-tools".to_string()
    )));
}

#[test]
fn exit_plan_tool_is_contributed_by_plan_mode_extension() {
    let default_registry = registry_for_mode(PolicyMode::Default);
    assert!(tool_names(&default_registry.tools).contains(&"exit_plan_mode".to_string()));

    let plan_registry = registry_for_mode(PolicyMode::Plan);
    assert!(tool_names(&plan_registry.tools).contains(&"exit_plan_mode".to_string()));
}

#[tokio::test]
async fn exit_plan_tool_requires_summary() {
    let result = ExitPlanModeTool
        .execute(context(PolicyMode::Plan), call(json!({ "summary": "" })))
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(result.data["error"]["kind"], "invalid_arguments");
}

#[tokio::test]
async fn exit_plan_tool_requires_plan_mode() {
    let result = ExitPlanModeTool
        .execute(
            context(PolicyMode::Default),
            call(json!({ "summary": "Build the approved change." })),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(result.data["error"]["kind"], "not_in_plan_mode");
}

#[tokio::test]
async fn exit_plan_tool_returns_pending_request_payload() {
    let result = ExitPlanModeTool
        .execute(
            context(PolicyMode::Plan),
            call(json!({
                "summary": "Build the approved change.",
                "next_steps": ["edit files", "run tests"],
                "target_mode": "accept_all"
            })),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(
        result.data["policy_exit_plan_request"]["target_mode"],
        "accept_all"
    );
    assert_eq!(
        result.data["policy_exit_plan_request"]["summary"],
        "Build the approved change."
    );
    assert!(
        result.data["policy_exit_plan_request"]["request_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
}

#[tokio::test]
async fn exit_plan_tool_accepts_legacy_accept_edits_target_mode() {
    let result = ExitPlanModeTool
        .execute(
            context(PolicyMode::Plan),
            call(json!({
                "summary": "Build the approved change.",
                "target_mode": "accept_edits"
            })),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(
        result.data["policy_exit_plan_request"]["target_mode"],
        "accept_all"
    );
}

fn registry_for_mode(mode: PolicyMode) -> roder_api::extension::ExtensionRegistry {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.install(PlanModeExtension::new(mode)).unwrap();
    builder.build().unwrap()
}

fn tool_names(
    contributors: &[std::sync::Arc<dyn roder_api::tools::ToolContributor>],
) -> Vec<String> {
    let mut registry = ToolRegistry::default();
    for contributor in contributors {
        contributor.contribute(&mut registry).unwrap();
    }
    registry.specs().into_iter().map(|spec| spec.name).collect()
}

fn context(mode: PolicyMode) -> ToolExecutionContext {
    ToolExecutionContext::new("thread", "turn", mode)
}

fn call(arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "call".to_string(),
        name: "exit_plan_mode".to_string(),
        raw_arguments: arguments.to_string(),
        arguments,
        thread_id: "thread".to_string(),
        turn_id: "turn".to_string(),
    }
}
