//! Claude Code inference provider.
//!
//! This crate uses the local `claude-agent-sdk` crate from
//! `/Users/pz/w/claude-agent-sdk-rust` to drive an authenticated local Claude
//! Code CLI process. Tests in this crate use a fake runner and do not spawn the
//! CLI unless the ignored live test is run explicitly.

mod extension;
mod options;
mod provider;

pub use extension::ClaudeCodeExtension;
pub use options::{build_options, parse_permission_mode, parse_setting_sources};
pub use provider::{ClaudeCodeConfig, ClaudeCodeEngine, ClaudeCodeRunner, SdkClaudeCodeRunner};
