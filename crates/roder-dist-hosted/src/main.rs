//! `roder-hosted` — first-party hosted multi-tenant service binary.
//!
//! Usage: `roder-hosted --config /etc/roder/hosted.toml`
//!
//! The config file is a `HostedConfig` TOML document (see
//! `docs/roder-hosted-service.md`). Secrets are env references only;
//! `RODER_HOSTED_*` env vars override listener/data-root settings.

use roder_config::hosted::HostedConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = match args.as_slice() {
        [flag, path] if flag == "--config" => path.clone(),
        _ => {
            eprintln!("usage: roder-hosted --config <hosted.toml>");
            std::process::exit(2);
        }
    };
    let text = std::fs::read_to_string(&config_path)
        .map_err(|error| anyhow::anyhow!("read {config_path}: {error}"))?;
    let config: HostedConfig =
        toml::from_str(&text).map_err(|error| anyhow::anyhow!("parse {config_path}: {error}"))?;

    let controller =
        roder_dist_hosted::launch(config, roder_dist_hosted::default_tenant_factory()).await?;
    eprintln!(
        "hosted gateway listening on ws://{}",
        controller.listen_addr
    );
    tokio::signal::ctrl_c().await?;
    controller.stop().await
}
