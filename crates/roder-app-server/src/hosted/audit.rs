//! Hosted audit records.
//!
//! Append-only, tenant-scoped, redaction-safe: records carry credential
//! ids and coarse reasons, never secrets or payloads. Backed by memory
//! with an optional JSONL file for durability.

use std::path::PathBuf;
use std::sync::Mutex;

use roder_api::identity::TenantId;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecord {
    /// `auth_ok`, `auth_failed`, `method_denied`, `request_policy_denied`,
    /// `rate_limited`, `service_account_created`,
    /// `service_account_revoked`, `admin_change`, `hook_change`.
    pub kind: String,
    /// Absent for failures that never resolved a tenant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<TenantId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Coarse machine-readable reason; never raw errors or secrets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Default)]
pub struct AuditLog {
    records: Mutex<Vec<AuditRecord>>,
    jsonl_path: Option<PathBuf>,
}

impl AuditLog {
    pub fn with_jsonl(path: PathBuf) -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            jsonl_path: Some(path),
        }
    }

    pub fn record(&self, record: AuditRecord) {
        if let (Some(path), Ok(line)) = (&self.jsonl_path, serde_json::to_string(&record)) {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{line}");
            }
        }
        self.records.lock().unwrap().push(record);
    }

    /// Records visible to a tenant (newest last).
    pub fn for_tenant(&self, tenant_id: &str) -> Vec<AuditRecord> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .filter(|record| record.tenant_id.as_deref() == Some(tenant_id))
            .cloned()
            .collect()
    }
}
