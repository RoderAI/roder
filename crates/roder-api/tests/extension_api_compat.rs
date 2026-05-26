use std::sync::Arc;

use roder_api::capabilities::{
    CapabilityDecision, CapabilityDenial, CapabilityGrant, CapabilityRequest,
};
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::inference::ProviderAuthType;
use roder_api::speech::{
    SpeechAudio, SpeechCapabilities, SpeechModelDescriptor, SpeechProviderContext,
    SpeechProviderMetadata, SpeechSynthesisCapabilities, SpeechSynthesisModelDescriptor,
    SpeechSynthesisRequest, SpeechSynthesisResult, SpeechSynthesizer, SpeechTranscriber,
    SpeechTranscriptionRequest, SpeechTranscriptionResult,
};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_api::tui_status::{StatusCell, StatusSegment, StatusStyle};
use semver::Version;

#[test]
fn registry_rejects_duplicate_extension_ids() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(StatusExtension::new("dup", "status-a"))
        .unwrap();

    let err = builder
        .install(StatusExtension::new("dup", "status-b"))
        .unwrap_err();

    assert!(err.to_string().contains("already installed"));
}

#[test]
fn registry_build_rejects_duplicate_services() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(StatusExtension::new("ext-a", "shared-status"))
        .unwrap();
    builder
        .install(StatusExtension::new("ext-b", "shared-status"))
        .unwrap();

    let err = build_err(builder);

    assert!(err.to_string().contains("duplicate provided service"));
}

#[test]
fn registry_build_rejects_manifest_service_without_installed_service() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.install(UninstalledServiceExtension).unwrap();

    let err = build_err(builder);

    assert!(
        err.to_string()
            .contains("no matching service was installed")
    );
}

#[test]
fn registry_build_rejects_incompatible_api_versions() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(StatusExtension::new("future-api", "status").with_api_version(">=99.0.0"))
        .unwrap();

    let err = build_err(builder);

    assert!(err.to_string().contains("unsupported API version"));
}

#[test]
fn registry_build_rejects_duplicate_tool_names() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(ToolExtension::new("tool-a", "provider-a", "same_tool"))
        .unwrap();
    builder
        .install(ToolExtension::new("tool-b", "provider-b", "same_tool"))
        .unwrap();

    let err = build_err(builder);

    assert!(err.to_string().contains("already registered"));
}

#[test]
fn registry_records_requested_and_granted_capabilities() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(
            StatusExtension::new("caps", "status").with_capabilities(vec![
                CapabilityRequest::new("fs.read.workspace"),
                CapabilityRequest::with_reason("network.http", "search extension"),
            ]),
        )
        .unwrap();
    builder.grant_capability("caps", CapabilityGrant::new("fs.read.workspace"));

    let registry = builder.build().unwrap();
    let statuses = registry.capability_statuses("caps");

    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[0].id, "fs.read.workspace");
    assert_eq!(statuses[0].decision, CapabilityDecision::Granted);
    assert_eq!(statuses[1].id, "network.http");
    assert_eq!(statuses[1].decision, CapabilityDecision::Requested);
    assert_eq!(statuses[1].reason.as_deref(), Some("search extension"));
}

#[test]
fn registry_installs_speech_transcribers() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.install(SpeechExtension).unwrap();

    let registry = builder.build().unwrap();

    assert!(registry.speech_transcriber("test-speech").is_some());
    assert!(
        registry
            .provided_services()
            .contains(&ProvidedService::SpeechTranscriber(
                "test-speech".to_string()
            ))
    );
}

#[test]
fn registry_installs_speech_synthesizers() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.install(SpeechSynthesisExtension).unwrap();

    let registry = builder.build().unwrap();

    assert!(registry.speech_synthesizer("test-synthesis").is_some());
    assert!(
        registry
            .provided_services()
            .contains(&ProvidedService::SpeechSynthesizer(
                "test-synthesis".to_string()
            ))
    );
}

#[test]
fn registry_build_rejects_denied_required_capability() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(
            StatusExtension::new("denied", "status")
                .with_capabilities(vec![CapabilityRequest::new("process.spawn")]),
        )
        .unwrap();
    builder.deny_capability(
        "denied",
        CapabilityDenial::new("process.spawn", "disabled in distribution"),
    );

    let err = build_err(builder);

    assert!(err.to_string().contains("requires denied capability"));
}

struct StatusExtension {
    id: &'static str,
    status_id: &'static str,
    api_version: &'static str,
    capabilities: Vec<CapabilityRequest>,
}

fn build_err(builder: ExtensionRegistryBuilder) -> anyhow::Error {
    match builder.build() {
        Ok(_) => panic!("expected registry build to fail"),
        Err(err) => err,
    }
}

impl StatusExtension {
    fn new(id: &'static str, status_id: &'static str) -> Self {
        Self {
            id,
            status_id,
            api_version: "0.1.0",
            capabilities: Vec::new(),
        }
    }

    fn with_api_version(mut self, api_version: &'static str) -> Self {
        self.api_version = api_version;
        self
    }

    fn with_capabilities(mut self, capabilities: Vec<CapabilityRequest>) -> Self {
        self.capabilities = capabilities;
        self
    }
}

impl RoderExtension for StatusExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: self.id.to_string(),
            name: self.id.to_string(),
            version: Version::new(0, 1, 0),
            api_version: self.api_version.to_string(),
            description: None,
            provides: vec![ProvidedService::StatusSegment(self.status_id.to_string())],
            required_capabilities: self.capabilities.clone(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.status_segment(StatusSegment::new(self.status_id, 10, 4, |_| StatusCell {
            text: "ok".to_string(),
            style: StatusStyle::Default,
            tooltip: None,
        }));
        Ok(())
    }
}

struct UninstalledServiceExtension;

impl RoderExtension for UninstalledServiceExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "uninstalled".to_string(),
            name: "uninstalled".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: None,
            provides: vec![ProvidedService::StatusSegment("missing".to_string())],
            required_capabilities: Vec::new(),
        }
    }

    fn install(&self, _registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        Ok(())
    }
}

struct ToolExtension {
    id: &'static str,
    provider_id: &'static str,
    tool_name: &'static str,
}

impl ToolExtension {
    fn new(id: &'static str, provider_id: &'static str, tool_name: &'static str) -> Self {
        Self {
            id,
            provider_id,
            tool_name,
        }
    }
}

impl RoderExtension for ToolExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: self.id.to_string(),
            name: self.id.to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: None,
            provides: vec![ProvidedService::ToolProvider(self.provider_id.to_string())],
            required_capabilities: Vec::new(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(TestToolContributor {
            provider_id: self.provider_id,
            tool_name: self.tool_name,
        }));
        Ok(())
    }
}

struct TestToolContributor {
    provider_id: &'static str,
    tool_name: &'static str,
}

impl ToolContributor for TestToolContributor {
    fn id(&self) -> roder_api::extension::ToolProviderId {
        self.provider_id.to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(TestToolExecutor {
            tool_name: self.tool_name,
        }))
    }
}

struct TestToolExecutor {
    tool_name: &'static str,
}

#[async_trait::async_trait]
impl ToolExecutor for TestToolExecutor {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.tool_name.to_string(),
            description: "test tool".to_string(),
            parameters: serde_json::json!({ "type": "object" }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            id: call.id,
            name: self.tool_name.to_string(),
            text: "ok".to_string(),
            data: serde_json::json!({}),
            is_error: false,
        })
    }
}

struct SpeechExtension;

impl RoderExtension for SpeechExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "speech".to_string(),
            name: "speech".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: None,
            provides: vec![ProvidedService::SpeechTranscriber(
                "test-speech".to_string(),
            )],
            required_capabilities: Vec::new(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.speech_transcriber(Arc::new(TestSpeechTranscriber));
        Ok(())
    }
}

struct TestSpeechTranscriber;

#[async_trait::async_trait]
impl SpeechTranscriber for TestSpeechTranscriber {
    fn id(&self) -> roder_api::extension::SpeechTranscriberId {
        "test-speech".to_string()
    }

    fn capabilities(&self) -> SpeechCapabilities {
        SpeechCapabilities {
            batch: true,
            streaming: false,
            diarization: false,
            timestamps: false,
            language_hints: true,
            prompt: true,
        }
    }

    fn metadata(&self) -> SpeechProviderMetadata {
        SpeechProviderMetadata {
            name: "Test Speech".to_string(),
            description: Some("test speech provider".to_string()),
            auth_type: ProviderAuthType::None,
            auth_label: None,
            auth_configured: Some(true),
            recommended: false,
            sort_order: 100,
        }
    }

    async fn list_models(
        &self,
        _ctx: SpeechProviderContext<'_>,
    ) -> anyhow::Result<Vec<SpeechModelDescriptor>> {
        Ok(vec![SpeechModelDescriptor {
            id: "test-transcribe".to_string(),
            name: "Test Transcribe".to_string(),
            description: None,
            capabilities: self.capabilities(),
        }])
    }

    async fn transcribe(
        &self,
        _ctx: SpeechProviderContext<'_>,
        request: SpeechTranscriptionRequest,
    ) -> anyhow::Result<SpeechTranscriptionResult> {
        assert_eq!(
            request.audio,
            SpeechAudio {
                bytes: b"audio".to_vec(),
                mime_type: "audio/wav".to_string(),
                filename: Some("clip.wav".to_string()),
            }
        );
        Ok(SpeechTranscriptionResult {
            text: "hello".to_string(),
            language: Some("en".to_string()),
            duration_millis: None,
            segments: Vec::new(),
            provider_response_id: None,
            metadata: serde_json::json!({}),
        })
    }
}

struct SpeechSynthesisExtension;

impl RoderExtension for SpeechSynthesisExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "speech-synthesis".to_string(),
            name: "speech synthesis".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: None,
            provides: vec![ProvidedService::SpeechSynthesizer(
                "test-synthesis".to_string(),
            )],
            required_capabilities: Vec::new(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.speech_synthesizer(Arc::new(TestSpeechSynthesizer));
        Ok(())
    }
}

struct TestSpeechSynthesizer;

#[async_trait::async_trait]
impl SpeechSynthesizer for TestSpeechSynthesizer {
    fn id(&self) -> roder_api::extension::SpeechSynthesizerId {
        "test-synthesis".to_string()
    }

    fn capabilities(&self) -> SpeechSynthesisCapabilities {
        SpeechSynthesisCapabilities {
            batch: true,
            streaming: false,
            builtin_voices: true,
            voice_design: false,
            voice_clone: false,
            prompt: true,
        }
    }

    fn metadata(&self) -> SpeechProviderMetadata {
        SpeechProviderMetadata::local("Test Speech Synthesis")
    }

    async fn list_models(
        &self,
        _ctx: SpeechProviderContext<'_>,
    ) -> anyhow::Result<Vec<SpeechSynthesisModelDescriptor>> {
        Ok(vec![SpeechSynthesisModelDescriptor {
            id: "test-tts".to_string(),
            name: "Test TTS".to_string(),
            description: None,
            capabilities: self.capabilities(),
        }])
    }

    async fn synthesize(
        &self,
        _ctx: SpeechProviderContext<'_>,
        request: SpeechSynthesisRequest,
    ) -> anyhow::Result<SpeechSynthesisResult> {
        Ok(SpeechSynthesisResult {
            audio: SpeechAudio {
                bytes: request.text.into_bytes(),
                mime_type: "audio/wav".to_string(),
                filename: None,
            },
            duration_millis: None,
            provider_response_id: None,
            metadata: serde_json::json!({}),
        })
    }
}
