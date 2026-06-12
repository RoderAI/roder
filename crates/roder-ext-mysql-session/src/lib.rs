//! MySQL port of the PostgreSQL session store
//! (`roder-ext-postgres-session`): tenant-scoped thread metadata, event,
//! item-event, extension-state, and context-artifact persistence.
//!
//! Differences from the PostgreSQL store are dialect-only: `?` placeholders,
//! `ON DUPLICATE KEY UPDATE`, JSON/LONGBLOB column types, and timestamps
//! stored as unix microseconds (BIGINT) to avoid MySQL TIMESTAMP range and
//! timezone pitfalls.

pub mod artifacts;
pub(crate) mod executor;
pub mod extension;
pub mod schema;
pub mod store;

pub use extension::*;
pub use store::{MysqlSessionConfig, MysqlSessionStore, redact_database_url, validate_tenant_id};
