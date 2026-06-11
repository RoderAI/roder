use super::*;

fn fixtures() -> Vec<ToolSearchEvalFixture> {
    let dir = default_tool_search_fixture_dir();
    let fixtures = load_tool_search_fixtures(&dir).expect("load tool-search fixtures");
    assert!(
        fixtures.len() >= 10,
        "expected the committed tool-search fixture matrix, found {} in {}",
        fixtures.len(),
        dir.display()
    );
    fixtures
}

#[test]
fn tool_search_fixture_suite_passes() {
    for fixture in fixtures() {
        assert_tool_search_fixture(&fixture)
            .unwrap_or_else(|err| panic!("fixture {} failed: {err:#}", fixture.id));
    }
}

#[test]
fn tool_search_fixture_matrix_covers_required_cases() {
    let fixtures = fixtures();
    let ids: Vec<&str> = fixtures.iter().map(|fixture| fixture.id.as_str()).collect();
    for required in [
        "anthropic-explicit-baseline",
        "anthropic-provider-native",
        "anthropic-unsupported-model-fail-closed",
        "openai-explicit-baseline",
        "openai-large-catalog-budget",
        "openai-provider-native",
        "openai-unsupported-model-fallback",
        "search-selection-denied-permission",
        "search-selection-malformed-results",
        "search-selection-subset-success",
        "search-selection-unknown-tool",
        "catalog-redaction",
    ] {
        assert!(ids.contains(&required), "missing required fixture {required}");
    }
    let providers_with_native: Vec<&str> = fixtures
        .iter()
        .filter(|fixture| fixture.mode == roder_api::inference::ToolSearchMode::ProviderNative)
        .map(|fixture| fixture.provider.as_str())
        .collect();
    assert!(providers_with_native.contains(&"openai"));
    assert!(providers_with_native.contains(&"anthropic"));
}

#[test]
fn tool_search_catalog_is_deterministic_and_limited() {
    let catalog_fixture = ToolSearchCatalogFixture {
        tools: vec![
            ToolSearchCatalogTool {
                name: "zeta".to_string(),
                description: "z".to_string(),
                parameters: None,
                internal_metadata: Default::default(),
            },
            ToolSearchCatalogTool {
                name: "alpha".to_string(),
                description: "a".to_string(),
                parameters: None,
                internal_metadata: Default::default(),
            },
            ToolSearchCatalogTool {
                name: "alpha".to_string(),
                description: "duplicate".to_string(),
                parameters: None,
                internal_metadata: Default::default(),
            },
        ],
        generated: Some(GeneratedCatalogFixture {
            count: 5,
            name_prefix: "gen".to_string(),
        }),
        max_items: Some(4),
    };

    let first = build_provider_safe_catalog(&catalog_fixture);
    let second = build_provider_safe_catalog(&catalog_fixture);

    assert_eq!(first, second, "catalog must be stable across runs");
    assert_eq!(first.len(), 4);
    let names: Vec<&str> = first.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "gen_0000", "gen_0001", "gen_0002"]);
    // Duplicate names keep the first (sorted) definition only.
    assert_eq!(first[0].description, "a");
}

#[test]
fn explicit_and_native_bodies_differ_only_in_tool_search_shape() {
    let fixtures = fixtures();
    let explicit = fixtures
        .iter()
        .find(|fixture| fixture.id == "openai-explicit-baseline")
        .unwrap();
    let native = fixtures
        .iter()
        .find(|fixture| fixture.id == "openai-provider-native")
        .unwrap();
    assert_eq!(
        explicit.catalog, native.catalog,
        "explicit/native comparison fixtures must share one task catalog"
    );

    let explicit_outcome = run_tool_search_fixture(explicit).unwrap();
    let native_outcome = run_tool_search_fixture(native).unwrap();
    let (ToolSearchOutcome::RequestMapped {
        body: explicit_body,
        deferred_tools: explicit_deferred,
        native_tool_search_entry: explicit_entry,
        ..
    }, ToolSearchOutcome::RequestMapped {
        body: native_body,
        deferred_tools: native_deferred,
        native_tool_search_entry: native_entry,
        ..
    }) = (&explicit_outcome, &native_outcome)
    else {
        panic!("expected mapped requests, got {explicit_outcome:?} / {native_outcome:?}");
    };

    assert_eq!(*explicit_deferred, 0);
    assert!(!explicit_entry);
    assert!(*native_deferred > 0);
    assert!(native_entry);
    assert_eq!(explicit_body["model"], native_body["model"]);
    assert_eq!(explicit_body["input"], native_body["input"]);
}

#[test]
fn large_catalog_native_mode_stays_within_prompt_budget() {
    let fixtures = fixtures();
    let fixture = fixtures
        .iter()
        .find(|fixture| fixture.id == "openai-large-catalog-budget")
        .unwrap();

    let outcome = run_tool_search_fixture(fixture).unwrap();
    let ToolSearchOutcome::RequestMapped {
        body,
        catalog_items,
        deferred_tools,
        native_tool_search_entry,
        ..
    } = &outcome
    else {
        panic!("expected mapped request, got {outcome:?}");
    };

    assert_eq!(*catalog_items, 200, "max_items must cap the catalog");
    assert_eq!(*deferred_tools, 200, "every catalog tool must defer");
    assert!(native_tool_search_entry);
    // Deferred schemas are still declared at the top level, but every
    // function entry is searchable instead of inline-required, which is the
    // provider contract for reducing live prompt pressure.
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 201);
    assert!(tools.iter().all(|tool| {
        tool["type"] != "function" || tool["defer_loading"] == serde_json::json!(true)
    }));
}
