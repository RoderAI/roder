use roder_api::tools::ScopedProcessRunner;

#[derive(Debug, Clone, Default)]
pub struct LocalProcessRunner;

impl ScopedProcessRunner for LocalProcessRunner {
    fn runner_name(&self) -> &str {
        "local-process"
    }
}
