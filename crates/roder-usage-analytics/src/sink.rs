//! Passive runtime recorder: an `EventSink` extension that projects
//! emitted runtime events into the analytics store.
//!
//! Registered through the standard extension registry, it rides the
//! runtime's bounded per-sink dispatch (roadmap phase 64): recording can
//! never block tool execution, provider requests, or turn progress, and a
//! broken store surfaces as redacted `extension.event_sink_failed` events.
//! Disabling analytics simply means not registering this extension.

use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::events::EventEnvelope;
use roder_api::extension::{
    EventSink, EventSinkId, ExtensionManifest, ExtensionRegistryBuilder, ProvidedService,
    RoderExtension,
};
use semver::Version;

use crate::ingest::AnalyticsIngestor;
use crate::store::AnalyticsStore;

pub const ANALYTICS_EXTENSION_ID: &str = "roder-usage-analytics";
pub const ANALYTICS_SINK_ID: &str = "usage-analytics";

pub struct UsageAnalyticsSink {
    store: Arc<AnalyticsStore>,
}

impl UsageAnalyticsSink {
    pub fn new(store: Arc<AnalyticsStore>) -> Self {
        Self { store }
    }
}

#[async_trait::async_trait]
impl EventSink for UsageAnalyticsSink {
    fn id(&self) -> EventSinkId {
        ANALYTICS_SINK_ID.to_string()
    }

    async fn handle_event(&self, envelope: &EventEnvelope) -> anyhow::Result<()> {
        AnalyticsIngestor::new(&self.store).ingest_event(envelope)
    }
}

/// Extension wrapper so hosts can install analytics via
/// `DefaultRegistryConfig::extra_extensions` or a plain registry builder.
pub struct UsageAnalyticsExtension {
    store: Arc<AnalyticsStore>,
}

impl UsageAnalyticsExtension {
    pub fn new(store: Arc<AnalyticsStore>) -> Self {
        Self { store }
    }
}

impl RoderExtension for UsageAnalyticsExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: ANALYTICS_EXTENSION_ID.to_string(),
            name: "Local Usage Analytics".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Projects local runtime events into the SQLite usage-analytics store.".to_string(),
            ),
            provides: vec![ProvidedService::EventSink(ANALYTICS_SINK_ID.to_string())],
            required_capabilities: vec![CapabilityRequest::new("events.read.all")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.event_sink(Arc::new(UsageAnalyticsSink::new(self.store.clone())));
        Ok(())
    }
}
