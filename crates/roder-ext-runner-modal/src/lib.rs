use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_ext_runner_hosted_common::{HostedRunnerProvider, HostedRunnerSpec};
use semver::Version;

pub const PROVIDER_ID: &str = "modal";
pub const TOKEN_ENV: &str = "MODAL_TOKEN_SECRET";
pub const BASE_URL_ENV: &str = "MODAL_RUNNER_BASE_URL";
pub const LIVE_ENV: &str = "RODER_LIVE_MODAL_RUNNER";

pub fn modal_runner_spec() -> HostedRunnerSpec {
    HostedRunnerSpec {
        provider_id: PROVIDER_ID,
        token_env: TOKEN_ENV,
        base_url_env: BASE_URL_ENV,
        default_base_url: "https://api.modal.com/roder/runner",
        live_env: LIVE_ENV,
    }
}

#[derive(Debug, Default)]
pub struct ModalRunnerExtension;

impl RoderExtension for ModalRunnerExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-runner-modal".to_string(),
            name: "Modal Runner".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Runs remote-runner sessions through Modal.".to_string()),
            provides: vec![ProvidedService::RemoteRunnerProvider(
                PROVIDER_ID.to_string(),
            )],
            required_capabilities: vec![
                CapabilityRequest::new("network.http"),
                CapabilityRequest::new(format!("secret.read.{TOKEN_ENV}")),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.remote_runner_provider(Arc::new(HostedRunnerProvider::new(modal_runner_spec())));
        Ok(())
    }
}
