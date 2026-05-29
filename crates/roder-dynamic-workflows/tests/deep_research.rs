use std::fs;

use roder_dynamic_workflows::{
    DeepResearchFixtureSearchProvider, WorkflowRunInput, WorkflowRuntimeErrorKind,
    WorkflowRuntimeOptions, WorkflowScriptRuntime, deep_research_arguments,
    deep_research_workflow_source, parse_workflow_definition,
};

#[test]
fn deep_research_definition_exposes_question_schema_and_phases() {
    let definition = parse_workflow_definition(
        deep_research_workflow_source(),
        &WorkflowRuntimeOptions::default(),
    )
    .unwrap();

    assert_eq!(definition.name, "deep-research");
    assert_eq!(
        definition.phases,
        vec!["scope", "parallel-research", "synthesis", "verification"]
    );
    assert_eq!(definition.arguments_schema["required"][0], "question");
    assert_eq!(definition.limits.max_concurrent_agents, 8);
}

#[test]
fn deep_research_fixture_provider_builds_offline_seed_results() {
    let provider = fixture_provider();
    let arguments = deep_research_arguments(
        "How should dynamic workflows coordinate verification agents?",
        Some(&provider),
    );

    assert_eq!(
        arguments["question"],
        "How should dynamic workflows coordinate verification agents?"
    );
    let seed_results = arguments["seedResults"].as_array().unwrap();
    assert!(!seed_results.is_empty());
    assert!(seed_results.iter().any(|result| {
        result["snippet"]
            .as_str()
            .unwrap()
            .contains("verifier lane")
    }));
}

#[test]
fn deep_research_runtime_uses_web_search_prompts_and_fixture_seed_results() {
    let provider = fixture_provider();
    let mut input = WorkflowRunInput::new("deep-research-run");
    input.arguments = deep_research_arguments(
        "How should dynamic workflows handle structured arguments?",
        Some(&provider),
    );

    let execution = WorkflowScriptRuntime::default()
        .run(deep_research_workflow_source(), input)
        .unwrap();

    assert_eq!(execution.definition.name, "deep-research");
    assert_eq!(
        execution.phases,
        vec!["scope", "parallel-research", "synthesis", "verification"]
    );
    assert!(execution.report.contains("# Deep research:"));
    assert_eq!(
        execution
            .agent_launches
            .iter()
            .filter(|launch| launch.role == "researcher")
            .count(),
        5
    );
    assert!(execution.agent_launches.iter().any(|launch| {
        launch.prompt.contains("canonical web-search tools")
            && launch.prompt.contains("Fixture seed results")
            && launch.prompt.contains("Structured workflow arguments")
    }));
}

#[test]
fn deep_research_refuses_without_web_search_or_fixture_results() {
    let mut input = WorkflowRunInput::new("deep-research-no-search");
    input.arguments = serde_json::json!({
        "question": "What should I research?",
        "webSearchAvailable": false
    });

    let error = WorkflowScriptRuntime::default()
        .run(deep_research_workflow_source(), input)
        .unwrap_err();

    assert_eq!(error.kind(), WorkflowRuntimeErrorKind::ScriptExecution);
    assert!(
        error
            .message()
            .contains("web-search capability or fixture seedResults"),
        "{}",
        error.message()
    );
}

fn fixture_provider() -> DeepResearchFixtureSearchProvider {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../evals/fixtures/dynamic-workflows/deep-research/corpus.json");
    DeepResearchFixtureSearchProvider::from_json_str(&fs::read_to_string(path).unwrap()).unwrap()
}
