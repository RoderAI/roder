mod artifact;
mod automation_worker;
mod automations;
pub mod client;
mod code_index;
mod command;
mod command_process;
mod discovery;
mod evals;
mod fs;
mod goals;
mod marketplaces;
#[cfg(test)]
mod method_manifest;
mod notifications;
mod processes;
mod protocol_contract;
pub mod remote;
mod retrieval;
mod search_index;
pub mod server;
mod skills;
pub mod transcript;

pub use automations::AppServerFeatureConfig;
pub use client::*;
pub use server::*;
