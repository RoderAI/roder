pub mod config;

use std::sync::Arc;
use roder_core::Runtime;
use roder_app_server::{AppServer, LocalAppClient};
use roder_tui::TuiApp;
use roder_extension_host::build_default_registry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = config::load_config().unwrap_or_default();
    
    let openai_key = cfg.providers.get("openai_responses").and_then(|p| p.api_key.clone());

    // 1. Build Extension Registry
    let registry = build_default_registry(openai_key)?;
    
    // We pick the responses engine as default, or fallback.
    let engine = registry.inference_engines.into_iter()
        .find(|e| e.id() == "openai-responses")
        .unwrap(); // For now we expect it to exist
    
    // 2. Start Core Runtime
    let runtime = Arc::new(Runtime::new(engine));
    
    // 3. Start App Server
    let app_server = Arc::new(AppServer::new(runtime));
    
    // 4. Start Client
    let client = LocalAppClient::new(app_server);
    
    let model = cfg.model.unwrap_or_else(|| "gpt-5.5".to_string());
    
    // 5. Run TUI
    let mut tui = TuiApp::new(client, model).await?;
    tui.run().await?;
    
    Ok(())
}
