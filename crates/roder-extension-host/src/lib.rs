use roder_api::extension::ExtensionRegistryBuilder;
use roder_ext_anthropic::AnthropicExtension;
use roder_ext_gemini::GeminiExtension;
use roder_ext_jsonl_session::JsonlSessionExtension;
use roder_ext_openai_chat_completions::OpenAiChatCompletionsExtension;
use roder_ext_openai_responses::OpenAiResponsesExtension;
use std::path::PathBuf;

pub fn build_default_registry(openai_api_key: Option<String>) -> anyhow::Result<ExtensionRegistryBuilder> {
    let mut builder = ExtensionRegistryBuilder::new();

    let dummy_key = "dummy".to_string();
    let openai_key = openai_api_key.unwrap_or_else(|| dummy_key.clone());

    builder.install(OpenAiChatCompletionsExtension::new(openai_key.clone()))?;
    builder.install(OpenAiResponsesExtension::new(openai_key.clone()))?;
    builder.install(AnthropicExtension::new(dummy_key.clone()))?;
    builder.install(GeminiExtension::new(dummy_key.clone()))?;

    let session_dir = PathBuf::from(".roder").join("sessions");
    builder.install(JsonlSessionExtension::new(session_dir))?;

    Ok(builder)
}