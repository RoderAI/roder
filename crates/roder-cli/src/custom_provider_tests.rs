use super::*;
use roder_api::provider_capabilities::{RemoteCompaction, ResponsesProviderCapabilities};

#[test]
fn custom_provider_registry_config_retains_capability_ceilings() {
    let capabilities = ResponsesProviderCapabilities {
        remote_compaction: Some(RemoteCompaction::V2),
        external_web_access: Some(false),
    };
    let mut config = roder_config::Config::default();
    config.providers.insert(
        "my-responses".into(),
        roder_config::ProviderConfig {
            base_url: Some("https://custom.example/v1".into()),
            capabilities,
            ..Default::default()
        },
    );
    let providers = custom_inference_providers(&config);
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, "my-responses");
    assert_eq!(providers[0].capabilities, capabilities);
}

#[test]
fn custom_provider_registry_config_retains_chat_protocol() {
    let mut config = roder_config::Config::default();
    config.providers.insert(
        "my-chat".into(),
        roder_config::ProviderConfig {
            base_url: Some("http://localhost/v1".into()),
            protocol: roder_config::CustomProviderProtocol::ChatCompletions,
            context_window: Some(16_384),
            tool_allowlist: vec!["read_file".into(), "grep".into()],
            ..Default::default()
        },
    );
    let providers = custom_inference_providers(&config);
    assert_eq!(providers.len(), 1);
    assert_eq!(
        providers[0].protocol,
        roder_config::CustomProviderProtocol::ChatCompletions
    );
    assert_eq!(providers[0].tool_allowlist, ["read_file", "grep"]);
    assert_eq!(providers[0].context_window, Some(16_384));
}
