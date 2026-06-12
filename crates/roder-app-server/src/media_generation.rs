//! `media/image/*` JSON-RPC handlers: provider/model listing and direct
//! image generation through the runtime's provider-neutral media service.

use roder_api::events::RoderEvent;
use roder_protocol::{
    JsonRpcError, MediaImageGenerateParams, MediaImageGenerateResult, MediaImageProvidersListResult,
};

use crate::server::{AppServer, internal_error};

impl AppServer {
    pub(crate) async fn handle_media_image_providers_list(
        &self,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let service = self.runtime.media_generation();
        Ok(serde_json::to_value(MediaImageProvidersListResult {
            default_provider: service.default_provider_id(),
            providers: service.provider_descriptors(),
        })
        .unwrap())
    }

    pub(crate) async fn handle_media_image_generate(
        &self,
        params: MediaImageGenerateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let service = self.runtime.media_generation();
        let response = service
            .generate_image(params.request)
            .await
            .map_err(internal_error)?;

        // Reuse the existing media event surface so clients observe direct
        // generations exactly like tool-driven ones. Direct generations are
        // not tied to a turn; the turn id stays empty.
        let thread_id = params.thread_id.unwrap_or_default();
        for output in &response.outputs {
            self.runtime
                .emit(RoderEvent::MediaArtifactCreated(
                    roder_api::events::MediaArtifactCreated {
                        thread_id: thread_id.clone(),
                        turn_id: String::new(),
                        artifact: output.artifact.clone(),
                        timestamp: time::OffsetDateTime::now_utc(),
                    },
                ))
                .await;
            self.runtime
                .emit(RoderEvent::MediaPreviewReady(
                    roder_api::events::MediaPreviewReady {
                        thread_id: thread_id.clone(),
                        turn_id: String::new(),
                        preview: output.preview.clone(),
                        timestamp: time::OffsetDateTime::now_utc(),
                    },
                ))
                .await;
        }

        Ok(serde_json::to_value(MediaImageGenerateResult { response }).unwrap())
    }
}
