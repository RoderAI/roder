//! `roder media ...` subcommands: image provider/model inspection and direct
//! image generation through the local app-server `media/image/*` methods.

use std::sync::Arc;

use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{JsonRpcRequest, MediaImageGenerateResult, MediaImageProvidersListResult};

use crate::{CliOptions, build_runtime_from_config, decode_response};

const USAGE: &str = "usage: roder media providers [--json]\n       roder media models [--provider <id>] [--json]\n       roder media generate <prompt> [--provider <id>] [--model <id>] [--count <n>] [--size <WxH>] [--aspect-ratio <w:h>] [--image-size <1K|2K|4K>] [--output-format <png|jpeg|webp>] [--json]";

pub(crate) async fn run_media_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("providers") => run_media_providers(&args[1..]).await,
        Some("models") => run_media_models(&args[1..]).await,
        Some("generate") => run_media_generate(&args[1..]).await,
        _ => anyhow::bail!("{USAGE}"),
    }
}

async fn local_client() -> anyhow::Result<LocalAppClient> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    Ok(LocalAppClient::new(Arc::new(AppServer::new(runtime))))
}

async fn list_providers() -> anyhow::Result<MediaImageProvidersListResult> {
    let client = local_client().await?;
    decode_response(
        client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: "media/image/providers/list".to_string(),
                params: None,
            })
            .await,
    )
}

async fn run_media_providers(args: &[String]) -> anyhow::Result<()> {
    let json = args.iter().any(|arg| arg == "--json");
    let result = list_providers().await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    for provider in result.providers {
        let default_marker = if provider.id == result.default_provider {
            " (default)"
        } else {
            ""
        };
        println!(
            "{}{}\t{}\tconfigured={}\timages={}",
            provider.id,
            default_marker,
            provider.display_name,
            provider.configured,
            provider.supports_images
        );
    }
    Ok(())
}

async fn run_media_models(args: &[String]) -> anyhow::Result<()> {
    let json = args.iter().any(|arg| arg == "--json");
    let provider_filter = flag_value(args, "--provider");
    let result = list_providers().await?;
    let models: Vec<_> = result
        .providers
        .iter()
        .filter(|provider| {
            provider_filter
                .as_deref()
                .is_none_or(|filter| provider.id == filter)
        })
        .flat_map(|provider| provider.image_models.iter())
        .collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&models)?);
        return Ok(());
    }
    for model in models {
        let mut tags = Vec::new();
        if model.is_default {
            tags.push("default");
        }
        if model.legacy {
            tags.push("legacy");
        }
        if model.supports_edit {
            tags.push("edit");
        }
        println!(
            "{}/{}\t{}\t{}",
            model.provider,
            model.id,
            model.display_name,
            tags.join(",")
        );
    }
    Ok(())
}

async fn run_media_generate(args: &[String]) -> anyhow::Result<()> {
    let Some(prompt) = args.first().filter(|arg| !arg.starts_with("--")) else {
        anyhow::bail!("{USAGE}");
    };
    let json = args.iter().any(|arg| arg == "--json");
    let mut params = serde_json::json!({ "prompt": prompt });
    let object = params.as_object_mut().unwrap();
    for (flag, key) in [
        ("--provider", "provider"),
        ("--model", "model"),
        ("--size", "size"),
        ("--aspect-ratio", "aspectRatio"),
        ("--image-size", "imageSize"),
        ("--output-format", "outputFormat"),
    ] {
        if let Some(value) = flag_value(args, flag) {
            object.insert(key.to_string(), serde_json::json!(value));
        }
    }
    if let Some(count) = flag_value(args, "--count") {
        object.insert(
            "count".to_string(),
            serde_json::json!(count.parse::<u32>()?),
        );
    }

    let client = local_client().await?;
    let result: MediaImageGenerateResult = decode_response(
        client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: "media/image/generate".to_string(),
                params: Some(params),
            })
            .await,
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    let response = result.response;
    for output in &response.outputs {
        println!(
            "{}\t{}\t{} bytes\t{}",
            output.artifact.id,
            output.artifact.mime_type,
            output.artifact.byte_size,
            output.artifact.store_path
        );
    }
    if let Some(revised_prompt) = response.revised_prompt.as_deref() {
        println!("revised prompt: {revised_prompt}");
    }
    Ok(())
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}
