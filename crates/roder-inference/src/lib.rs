pub use roder_api::inference::*;

pub fn provider_supports_tools(capabilities: &InferenceCapabilities) -> bool {
    capabilities.tool_calls
}

pub fn provider_supports_streaming(capabilities: &InferenceCapabilities) -> bool {
    capabilities.streaming
}
