//! MySQL port of the PostgreSQL session store
//! (`roder-ext-postgres-session`): tenant-scoped thread metadata, event,
//! item-event, extension-state, and context-artifact persistence.
//!
//! MySQL uses `?` placeholders,
//! `ON DUPLICATE KEY UPDATE`, JSON/LONGBLOB column types, and timestamps
//! stored as unix microseconds (BIGINT) to avoid MySQL TIMESTAMP range and
//! timezone pitfalls.

pub mod artifacts;
pub mod config;
mod event_log;
pub(crate) mod executor;
pub mod extension;
pub mod ownership;
pub mod schema;
pub mod store;
mod write_guard;

pub use extension::*;
pub use config::{MysqlSessionConfig, redact_database_url, validate_tenant_id};
pub use store::MysqlSessionStore;
