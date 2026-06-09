use roder_api::extension::{ExtensionRegistryBuilder, RoderExtension};
use roder_ext_inference_router::{LOCAL_INFERENCE_ROUTER_ID, LocalInferenceRouterExtension};

#[test]
fn extension_installs_local_router_service() {
    let extension = LocalInferenceRouterExtension::default();
    let manifest = extension.manifest();

    assert!(manifest.provides.iter().any(|service| matches!(
        service,
        roder_api::extension::ProvidedService::InferenceRouter(id)
            if id == LOCAL_INFERENCE_ROUTER_ID
    )));

    let mut builder = ExtensionRegistryBuilder::new();
    builder.install(extension).unwrap();
    let registry = builder.build().unwrap();

    assert!(
        registry
            .inference_router(LOCAL_INFERENCE_ROUTER_ID)
            .is_some()
    );
}
