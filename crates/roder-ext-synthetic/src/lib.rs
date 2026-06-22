mod extension;
mod provider;

pub use extension::SyntheticExtension;
pub use provider::{
    API_KEY_ALIASES, API_KEY_ENV, BASE_URL_ALIASES, DEFAULT_BASE_URL, PROVIDER_NAME,
    SyntheticConfig, SyntheticInferenceEngine,
};
