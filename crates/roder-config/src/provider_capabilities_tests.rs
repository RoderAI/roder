use crate::Config;
use roder_api::provider_capabilities::{RemoteCompaction, ResponsesProviderCapabilities};

#[test]
fn custom_responses_capabilities_round_trip_and_preserve_omitted_defaults() {
    let config: Config = toml::from_str(
        r#"
[providers.custom]
base_url = "https://custom.example/v1"
[providers.custom.capabilities]
remote_compaction = "v2"
external_web_access = false

[providers.ordinary]
base_url = "https://ordinary.example/v1"
"#,
    )
    .unwrap();
    let expected = ResponsesProviderCapabilities {
        remote_compaction: Some(RemoteCompaction::V2),
        external_web_access: Some(false),
    };
    assert_eq!(config.providers["custom"].capabilities, expected);
    assert_eq!(
        config.providers["ordinary"].capabilities,
        ResponsesProviderCapabilities::default()
    );
    let encoded = toml::to_string(&config).unwrap();
    let decoded: Config = toml::from_str(&encoded).unwrap();
    assert_eq!(decoded.providers["custom"].capabilities, expected);
}

#[test]
fn custom_capabilities_reject_unknown_protocols_and_misspellings() {
    for input in [
        r#"[providers.custom.capabilities]
remote_compaction = "v1""#,
        r#"[providers.custom.capabilities]
external_web_acess = false"#,
    ] {
        assert!(toml::from_str::<Config>(input).is_err());
    }
}
