pub mod config;
pub mod extension;
mod policy;
mod profiler;
mod scoring;
mod signals;

pub use config::LocalInferenceRouterConfig;
pub use extension::{
    LOCAL_INFERENCE_ROUTER_ID, LocalInferenceRouter, LocalInferenceRouterExtension,
};
