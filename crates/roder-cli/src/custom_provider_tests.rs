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
