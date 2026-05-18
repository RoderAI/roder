use std::fs;

use roder_api::distribution::Profile;
use roder_configure::catalog::Catalog;
use roder_configure::profile::{BUILT_IN_PROFILES, ProfileExt, built_in_profiles};

#[test]
fn profiles_loads_strict_toml_from_path() {
    let path = std::env::temp_dir().join(format!(
        "roder-profile-{}-{}.toml",
        std::process::id(),
        "strict"
    ));
    fs::write(
        &path,
        r#"
id = "test"
description = "test profile"

[distribution]
name = "test-roder"
version = "0.1.0"
include_tui = true
include_app_server = false
include_cli = true
extensions = ["jsonl-session"]
default_session_store = "jsonl-session"
"#,
    )
    .unwrap();

    let profile = Profile::load(&path).unwrap();

    assert_eq!(profile.id, "test");
    assert_eq!(profile.manifest.extensions, vec!["jsonl-session"]);
    let _ = fs::remove_file(path);
}

#[test]
fn profiles_reject_unknown_toml_fields() {
    let err = Profile::from_toml(
        r#"
id = "bad"
description = "bad profile"
unknown = true

[distribution]
name = "bad-roder"
version = "0.1.0"
include_tui = true
include_app_server = false
include_cli = true
"#,
    )
    .unwrap_err();

    assert!(err.to_string().contains("unknown"));
}

#[test]
fn profiles_builtin_profiles_validate_against_workspace_catalog() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let profiles = built_in_profiles().unwrap();

    assert_eq!(profiles.len(), BUILT_IN_PROFILES.len());
    for profile in profiles {
        let report = profile.validate(&catalog).unwrap();
        if profile.id == "openai-only" {
            assert_eq!(
                report.required_env.get("openai-responses").unwrap(),
                &vec!["OPENAI_API_KEY".to_string()]
            );
        }
    }
}

#[test]
fn profiles_validation_points_to_unknown_extension() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let profile = Profile::from_toml(
        r#"
id = "bad"
description = "bad profile"

[distribution]
name = "bad-roder"
version = "0.1.0"
include_tui = true
include_app_server = false
include_cli = true
extensions = ["does-not-exist"]
"#,
    )
    .unwrap();

    let err = profile.validate(&catalog).unwrap_err();

    assert!(err.to_string().contains("does-not-exist"));
}

#[test]
fn profiles_validation_points_to_disabled_required_capability() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let profile = Profile::from_toml(
        r#"
id = "bad-capability"
description = "bad capability profile"

[distribution]
name = "bad-capability-roder"
version = "0.1.0"
include_tui = true
include_app_server = false
include_cli = true
extensions = ["openai-responses", "jsonl-session"]
default_provider = "openai-responses"
default_session_store = "jsonl-session"

[distribution.config_overrides]
disabled_capabilities = ["secret.read.OPENAI_API_KEY"]
"#,
    )
    .unwrap();

    let err = profile.validate(&catalog).unwrap_err();

    assert!(err.to_string().contains("secret.read.OPENAI_API_KEY"));
}
