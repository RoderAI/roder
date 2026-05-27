use roder_api::catalog::{PROVIDER_CURSOR, models_for_provider};
use roder_api::inference::ModelDescriptor;

pub fn fallback_models() -> Vec<ModelDescriptor> {
    models_for_provider(PROVIDER_CURSOR, false)
}
