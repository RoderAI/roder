use roder_api::extension::ExtensionRegistryBuilder;
use roder_config::cua::CuaConfig;

pub(super) fn install(
    builder: &mut ExtensionRegistryBuilder,
    config: Option<&CuaConfig>,
) -> anyhow::Result<()> {
    if let Some(config) = config.filter(|config| config.enabled) {
        builder.install(roder_ext_cua::CuaExtension::new(config.clone()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::extension::ProvidedService;
    #[test]
    fn desktop_tools_are_explicitly_enabled() {
        for enabled in [false, true] {
            let mut builder = ExtensionRegistryBuilder::new();
            install(
                &mut builder,
                Some(&CuaConfig {
                    enabled,
                    ..Default::default()
                }),
            )
            .unwrap();
            let registry = builder.build().unwrap();
            assert_eq!(
                registry
                    .provided_services()
                    .contains(&ProvidedService::ToolProvider("cua".into())),
                enabled
            );
        }
    }
}
