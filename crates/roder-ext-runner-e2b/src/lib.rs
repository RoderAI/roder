use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_ext_runner_hosted_common::{HostedRunnerProvider, HostedRunnerSpec};
use semver::Version;

pub const PROVIDER_ID: &str = "e2b";
pub const TOKEN_ENV: &str = "E2B_API_KEY";
pub const BASE_URL_ENV: &str = "E2B_RUNNER_BASE_URL";
pub const LIVE_ENV: &str = "RODER_LIVE_E2B_RUNNER";

pub fn e2b_runner_spec() -> HostedRunnerSpec {
    HostedRunnerSpec {
        provider_id: PROVIDER_ID,
        token_env: TOKEN_ENV,
        base_url_env: BASE_URL_ENV,
        default_base_url: "https://api.e2b.dev/roder/runner",
        live_env: LIVE_ENV,
    }
}

#[derive(Debug, Default)]
pub struct E2bRunnerExtension;

impl RoderExtension for E2bRunnerExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-runner-e2b".to_string(),
            name: "E2B Runner".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Runs remote-runner sessions through E2B.".to_string()),
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
        registry.remote_runner_provider(Arc::new(HostedRunnerProvider::new(e2b_runner_spec())));
        Ok(())
    }
}
