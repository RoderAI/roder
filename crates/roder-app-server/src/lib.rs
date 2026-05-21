mod artifact;
mod automation_worker;
mod automations;
pub mod client;
mod code_index;
mod command;
mod command_process;
mod desktop_contract;
mod discovery;
mod evals;
mod fs;
mod marketplaces;
#[cfg(test)]
mod method_manifest;
mod notifications;
mod processes;
pub mod remote;
mod retrieval;
mod search_index;
pub mod server;
mod skills;
pub mod transcript;

pub use automations::AppServerFeatureConfig;
pub use client::*;
pub use server::*;
