pub use roder_api::extension::RoderExtension;
pub use roder_extension_host::InferenceProviderSelection;

#[cfg(not(test))]
include!("main.rs");

#[cfg(not(test))]
pub fn run() -> anyhow::Result<()> {
    main()
}

#[cfg(test)]
pub fn run() -> anyhow::Result<()> {
    Ok(())
}

/// Composition options for out-of-tree distribution binaries that embed the
/// full roder CLI.
#[derive(Default)]
pub struct DistributionOptions {
    /// Installed into every registry the process builds (TUI, exec,
    /// app-server, ...) after the built-in extension set.
    pub extra_extensions: Vec<std::sync::Arc<dyn RoderExtension>>,
    /**
     * Pins the API-key inference-provider set for every registry the process
     * builds. `None` keeps the stock behavior of declaring the set from the
     * credentials resolved at startup (env or user config).
     */
    pub inference_providers: Option<Vec<InferenceProviderSelection>>,
}

/// Runs the full roder CLI (args come from `std::env::args`) with the given
/// distribution extensions registered for the whole process.
pub fn run_distribution(options: DistributionOptions) -> anyhow::Result<()> {
    roder_extension_host::set_distribution_extensions(options.extra_extensions)?;
    if let Some(providers) = options.inference_providers {
        roder_extension_host::set_distribution_inference_providers(providers)?;
    }
    run()
}
