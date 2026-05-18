use roder_api::distribution::ExtensionCategory;
use roder_configure::catalog::Catalog;
use roder_configure::profile::built_in_profile;
use roder_configure::tui::picker::PickerState;
use roder_configure::tui::wizard::{WizardState, WizardStep, category_ids};

#[test]
fn tui_wizard_state_moves_forward_and_back_without_losing_edits() {
    let profile = built_in_profile("openai-only").unwrap().unwrap();
    let mut state = WizardState::new(Some(profile));

    state.next();
    state.manifest.name = "edited-roder".to_string();
    state.next();
    state.next();
    state.back();
    state.back();

    assert_eq!(state.step, WizardStep::NameVersion);
    assert_eq!(state.manifest.name, "edited-roder");
}

#[test]
fn tui_wizard_surfaces_required_env_summary() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let profile = built_in_profile("openai-only").unwrap().unwrap();
    let state = WizardState::new(Some(profile));

    let confirmation = state.confirmation(&catalog).unwrap();

    assert_eq!(
        confirmation.required_env.get("openai-responses").unwrap(),
        &vec!["OPENAI_API_KEY".to_string()]
    );
}

#[test]
fn tui_wizard_conflict_errors_are_visible_before_generation() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let mut profile = built_in_profile("openai-only").unwrap().unwrap();
    profile
        .manifest
        .extensions
        .push("does-not-exist".to_string());
    let state = WizardState::new(Some(profile));

    let err = state.confirmation(&catalog).unwrap_err();

    assert!(err.contains("does-not-exist"));
}

#[test]
fn tui_wizard_picker_filters_by_category_and_query_with_help() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let entries = catalog.entries().collect::<Vec<_>>();
    let picker = PickerState {
        query: "openai".to_string(),
        category: Some(ExtensionCategory::InferenceEngine),
        selected_index: 0,
    };

    let matches = picker.matches(&entries);

    assert!(
        matches
            .iter()
            .any(|entry| entry.entry.id == "openai-responses")
    );
    assert!(
        matches
            .iter()
            .all(|entry| entry.entry.category == ExtensionCategory::InferenceEngine)
    );
    assert!(PickerState::help(matches[0]).contains(&matches[0].entry.description));
}

#[test]
fn tui_wizard_category_helpers_power_multiselect_steps() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();

    let inference = category_ids(&catalog, ExtensionCategory::InferenceEngine);

    assert!(inference.contains(&"openai-responses".to_string()));
    assert!(inference.contains(&"anthropic".to_string()));
}
