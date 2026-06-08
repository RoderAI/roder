use crate::client::SpritesClient;
use crate::config::SpritesConfig;

pub async fn apply_configured_policies(
    client: &SpritesClient,
    sprite_name: &str,
    config: &SpritesConfig,
) -> anyhow::Result<()> {
    if let Some(policy) = &config.network_policy {
        client
            .put_policy(sprite_name, "/policy/network", policy)
            .await?;
    }
    if let Some(policy) = &config.privileges_policy {
        client
            .put_policy(sprite_name, "/policy/privileges", policy)
            .await?;
    }
    if let Some(policy) = &config.resources_policy {
        client
            .put_policy(sprite_name, "/policy/resources", policy)
            .await?;
    }
    if !config.connectors.is_empty() {
        for connector in &config.connectors {
            client
                .put_policy(sprite_name, "/connectors/provision", connector)
                .await?;
        }
    }
    Ok(())
}
