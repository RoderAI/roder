use roder_api::catalog::PROVIDER_FIREWORKS;
use roder_api::extension::{ExtensionRegistryBuilder, RoderExtension};
use roder_api::inference::InferenceProviderContext;
use roder_ext_fireworks::{FireworksConfig, FireworksExtension};

#[test]
fn installs_fireworks_engine_with_offline_metadata() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(FireworksExtension::new(FireworksConfig::default()))
        .unwrap();
    let registry = builder.build().unwrap();
    let engine = registry
        .inference_engine(PROVIDER_FIREWORKS)
        .expect("fireworks engine registered");

    let metadata = engine.metadata();
    assert_eq!(metadata.name, "Fireworks AI");
    assert_eq!(metadata.auth_label.as_deref(), Some("FIREWORKS_API_KEY"));
    assert_eq!(metadata.auth_configured, Some(false));
}

#[tokio::test]
async fn list_models_returns_account_scoped_fallback_without_credentials() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(FireworksExtension::new(FireworksConfig::default()))
        .unwrap();
    let registry = builder.build().unwrap();
    let engine = registry.inference_engine(PROVIDER_FIREWORKS).unwrap();

    let models = engine
        .list_models(InferenceProviderContext {
            provider_id: PROVIDER_FIREWORKS,
        })
        .await
        .unwrap();

    assert!(models.iter().any(|model| {
        model.id == "accounts/fireworks/models/qwen3-235b-a22b" && model.name == "Qwen3 235B A22B"
    }));
}

#[test]
fn manifest_declares_fireworks_capabilities_without_secret_until_configured() {
    let without_key = FireworksExtension::new(FireworksConfig::default()).manifest();
    assert!(
        without_key
            .required_capabilities
            .iter()
            .any(|capability| capability.id == "network.api.fireworks.ai")
    );
    assert!(
        !without_key
            .required_capabilities
            .iter()
            .any(|capability| capability.id == "secret.read.FIREWORKS_API_KEY")
    );

    let with_key = FireworksExtension::new(FireworksConfig {
        api_key: Some("secret".to_string()),
        ..FireworksConfig::default()
    })
    .manifest();
    assert!(
        with_key
            .required_capabilities
            .iter()
            .any(|capability| capability.id == "secret.read.FIREWORKS_API_KEY")
    );
}
