use crate::ProviderKeys;
use roder_extension_host::InferenceProviderSelection;

pub(super) fn stock_inference_providers(
    keys: &ProviderKeys,
) -> Vec<roder_extension_host::InferenceProviderSelection> {
    providers_from_keys(
        keys,
        std::env::var("KIMI_CODE_API_KEY").is_ok()
            || std::env::var("RODER_KIMI_CODE_API_KEY").is_ok()
            || kimi_code_has_stored_tokens(),
    )
}

#[cfg(feature = "kimi-code")]
fn kimi_code_has_stored_tokens() -> bool {
    roder_ext_kimi_code::has_stored_tokens()
}

#[cfg(not(feature = "kimi-code"))]
fn kimi_code_has_stored_tokens() -> bool {
    false
}

pub(super) fn providers_from_keys(
    keys: &ProviderKeys,
    kimi_authenticated: bool,
) -> Vec<InferenceProviderSelection> {
    let mut providers = Vec::new();
    if keys.anthropic.is_some() {
        providers.push(InferenceProviderSelection::Anthropic);
    }
    if keys.openai.is_some() {
        providers.push(InferenceProviderSelection::OpenAi);
    }
    if keys.gemini.is_some() {
        providers.push(InferenceProviderSelection::Gemini);
    }
    if keys.vertex_credentials_path.is_some() || keys.vertex_credentials_json.is_some() {
        providers.push(InferenceProviderSelection::Vertex);
    }
    if keys.xai.is_some() {
        providers.push(InferenceProviderSelection::Xai);
    }
    if keys.kimi_code.is_some() || kimi_authenticated {
        providers.push(InferenceProviderSelection::KimiCode);
    }
    providers
}
