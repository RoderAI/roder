use std::fs;

use roder_api::dynamic_workflows::WorkflowRunLimits;
use roder_dynamic_workflows::{
    WorkflowCheckpoint, WorkflowCheckpointStore, WorkflowRunInput, WorkflowRuntimeErrorKind,
    WorkflowRuntimeOptions, WorkflowScriptRuntime, parse_workflow_definition,
};

fn runtime_with_limits(limits: WorkflowRunLimits) -> WorkflowScriptRuntime {
    WorkflowScriptRuntime::new(WorkflowRuntimeOptions {
        limits,
        ..WorkflowRuntimeOptions::default()
    })
}

#[test]
fn script_fixture_produces_report_with_ordered_fake_host_agents() {
    let runtime = WorkflowScriptRuntime::default();
    let execution = runtime
        .run(
            r#"
workflow.define({
  name: "audit",
  description: "Audit scoped files",
  argumentsSchema: { type: "object" },
  phases: ["Scout"],
  limits: { maxAgentsPerRun: 4 }
}, async (ctx) => {
  ctx.phase.start("Scout");
  const findings = await ctx.agents.reduce("scout", ["api", "core"], (target) => ({
    lane: "scout",
    description: `inspect ${target}`,
    prompt: `Inspect ${target}`,
    output: `finding:${target}`
  }), (acc, item) => acc.concat([item.output]), []);
  const vote = ctx.results.vote(findings);
  ctx.checkpoint.save("findings", findings);
  return ctx.report.markdown([vote.winner].concat(findings));
});
"#,
            WorkflowRunInput::new("run-1"),
        )
        .unwrap();

    assert_eq!(execution.definition.name, "audit");
    assert_eq!(execution.definition.phases, vec!["Scout"]);
    assert_eq!(execution.phases, vec!["Scout"]);
    assert_eq!(execution.agent_launches.len(), 2);
    assert_eq!(execution.agent_launches[0].input, "api");
    assert_eq!(execution.agent_launches[1].input, "core");
    assert_eq!(execution.report, "finding:api\nfinding:api\nfinding:core");
    assert_eq!(execution.checkpoints[0].key, "findings");
}

#[test]
fn script_parser_accepts_partial_limits_and_defaults_the_rest() {
    let options = WorkflowRuntimeOptions::default();
    let definition = parse_workflow_definition(
        r#"
workflow.define({
  name: "review",
  hostApiVersion: 1,
  limits: { maxAgentsPerRun: 2 }
}, () => "ok");
"#,
        &options,
    )
    .unwrap();

    assert_eq!(definition.name, "review");
    assert_eq!(definition.limits.max_agents_per_run, 2);
    assert_eq!(
        definition.limits.default_agent_timeout_seconds,
        WorkflowRunLimits::default().default_agent_timeout_seconds
    );
}

#[test]
fn ambient_api_usage_is_denied_before_execution() {
    let runtime = WorkflowScriptRuntime::default();
    let error = runtime
        .run(
            r#"
workflow.define({ name: "network" }, () => fetch("https://example.com"));
"#,
            WorkflowRunInput::new("run-ambient"),
        )
        .unwrap_err();

    assert_eq!(error.kind(), WorkflowRuntimeErrorKind::DeniedAmbientApi);
    assert!(error.message().contains("network access"));
}

#[test]
fn runtime_limit_stops_agent_launches_beyond_configured_cap() {
    let limits = WorkflowRunLimits {
        max_agents_per_run: 1,
        ..WorkflowRunLimits::default()
    };
    let runtime = runtime_with_limits(limits);
    let error = runtime
        .run(
            r#"
workflow.define({ name: "too-many" }, async (ctx) => {
  await ctx.agents.map("scout", ["one", "two"], (target) => ({ prompt: target }));
  return "unreachable";
});
"#,
            WorkflowRunInput::new("run-limit"),
        )
        .unwrap_err();

    assert_eq!(error.kind(), WorkflowRuntimeErrorKind::LimitExceeded);
    assert!(error.message().contains("maxAgentsPerRun"));
}

#[test]
fn abort_signal_stops_before_first_host_launch() {
    let runtime = WorkflowScriptRuntime::default();
    let mut input = WorkflowRunInput::new("run-abort");
    input.abort_before_start = true;

    let error = runtime
        .run(
            r#"
workflow.define({ name: "abortable" }, async (ctx) => {
  await ctx.agents.run("scout", { prompt: "should not launch" });
  return "unreachable";
});
"#,
            input,
        )
        .unwrap_err();

    assert_eq!(error.kind(), WorkflowRuntimeErrorKind::Aborted);
}

#[test]
fn checkpoint_store_persists_jsonl_records() {
    let root = std::env::temp_dir().join(format!(
        "roder-dynamic-workflow-store-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let store = WorkflowCheckpointStore::new(&root);
    let checkpoint = WorkflowCheckpoint {
        key: "phase-1".to_string(),
        value: serde_json::json!({ "complete": true }),
        byte_count: 17,
    };

    store.append_checkpoint("run-store", &checkpoint).unwrap();
    let loaded = store.read_checkpoints("run-store").unwrap();

    assert_eq!(loaded, vec![checkpoint]);
    fs::remove_dir_all(root).unwrap();
}
