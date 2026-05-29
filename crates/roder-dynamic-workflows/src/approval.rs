use std::collections::{BTreeSet, HashMap};

use roder_api::dynamic_workflows::{WorkflowApprovalDecision, WorkflowConsent, WorkflowScriptHash};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowApprovalScope {
    RunOnce,
    ScriptAndWorkspace,
}

impl WorkflowApprovalScope {
    pub fn from_decision(decision: WorkflowApprovalDecision) -> Option<Self> {
        match decision {
            WorkflowApprovalDecision::RunOnce => Some(Self::RunOnce),
            WorkflowApprovalDecision::AlwaysForScriptAndWorkspace => Some(Self::ScriptAndWorkspace),
            WorkflowApprovalDecision::Deny => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowConsentKey {
    pub script_hash: WorkflowScriptHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub scope: WorkflowApprovalScope,
}

impl WorkflowConsentKey {
    pub fn new(
        script_hash: impl Into<WorkflowScriptHash>,
        workspace: Option<impl Into<String>>,
        source_path: Option<impl Into<String>>,
        scope: WorkflowApprovalScope,
    ) -> Self {
        Self {
            script_hash: script_hash.into(),
            workspace: workspace.map(Into::into),
            source_path: source_path.map(Into::into),
            scope,
        }
    }

    pub fn reusable(
        script_hash: impl Into<WorkflowScriptHash>,
        workspace: Option<impl Into<String>>,
        source_path: Option<impl Into<String>>,
    ) -> Self {
        Self::new(
            script_hash,
            workspace,
            source_path,
            WorkflowApprovalScope::ScriptAndWorkspace,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoredWorkflowConsent {
    pub key: WorkflowConsentKey,
    pub consent: WorkflowConsent,
}

impl StoredWorkflowConsent {
    pub fn approved_capabilities(&self) -> BTreeSet<&str> {
        self.consent
            .approved_capabilities
            .iter()
            .map(String::as_str)
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowConsentStore {
    entries: HashMap<WorkflowConsentKey, StoredWorkflowConsent>,
}

impl WorkflowConsentStore {
    pub fn record(
        &mut self,
        key: WorkflowConsentKey,
        approved_capabilities: Vec<String>,
        decided_at: OffsetDateTime,
        expires_at: Option<OffsetDateTime>,
    ) -> StoredWorkflowConsent {
        let consent = WorkflowConsent {
            script_hash: key.script_hash.clone(),
            workspace: key.workspace.clone(),
            decision: match key.scope {
                WorkflowApprovalScope::RunOnce => WorkflowApprovalDecision::RunOnce,
                WorkflowApprovalScope::ScriptAndWorkspace => {
                    WorkflowApprovalDecision::AlwaysForScriptAndWorkspace
                }
            },
            approved_capabilities,
            decided_at,
            expires_at,
        };
        let stored = StoredWorkflowConsent { key, consent };
        self.entries.insert(stored.key.clone(), stored.clone());
        stored
    }

    pub fn reusable_consent(
        &self,
        key: &WorkflowConsentKey,
        now: OffsetDateTime,
        requested_capabilities: &[String],
    ) -> Option<&StoredWorkflowConsent> {
        if key.scope != WorkflowApprovalScope::ScriptAndWorkspace {
            return None;
        }
        let stored = self.entries.get(key)?;
        if stored
            .consent
            .expires_at
            .is_some_and(|expires_at| expires_at <= now)
        {
            return None;
        }
        let approved = stored.approved_capabilities();
        requested_capabilities
            .iter()
            .all(|capability| approved.contains(capability.as_str()))
            .then_some(stored)
    }

    pub fn get(&self, key: &WorkflowConsentKey) -> Option<&StoredWorkflowConsent> {
        self.entries.get(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn workflow_script_hash(source: &str) -> WorkflowScriptHash {
    let digest = Sha256::digest(source.as_bytes());
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reusable_consent_is_keyed_by_script_workspace_source_and_scope() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let hash = workflow_script_hash("workflow.define({name:'audit'}, async () => {})");
        let key = WorkflowConsentKey::reusable(
            hash.clone(),
            Some("/workspace"),
            Some(".agents/workflows/audit.workflow.js"),
        );
        let mut store = WorkflowConsentStore::default();
        store.record(
            key.clone(),
            vec!["childAgents".to_string(), "checkpoints".to_string()],
            now,
            None,
        );

        assert!(
            store
                .reusable_consent(&key, now, &["childAgents".to_string()])
                .is_some()
        );

        let different_source = WorkflowConsentKey::reusable(
            hash,
            Some("/workspace"),
            Some(".agents/workflows/other.workflow.js"),
        );
        assert!(
            store
                .reusable_consent(&different_source, now, &["childAgents".to_string()])
                .is_none()
        );
    }

    #[test]
    fn reusable_consent_does_not_expand_capabilities_or_expired_grants() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let key = WorkflowConsentKey::reusable("hash", Some("/workspace"), None::<String>);
        let mut store = WorkflowConsentStore::default();
        store.record(
            key.clone(),
            vec!["childAgents".to_string()],
            now,
            Some(now + time::Duration::seconds(5)),
        );

        assert!(
            store
                .reusable_consent(&key, now, &["shell".to_string()])
                .is_none()
        );
        assert!(
            store
                .reusable_consent(
                    &key,
                    now + time::Duration::seconds(6),
                    &["childAgents".to_string()]
                )
                .is_none()
        );
    }

    #[test]
    fn run_once_approval_is_not_reusable_consent() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let key = WorkflowConsentKey::new(
            "hash",
            Some("/workspace"),
            None::<String>,
            WorkflowApprovalScope::RunOnce,
        );
        let mut store = WorkflowConsentStore::default();
        store.record(key.clone(), vec!["childAgents".to_string()], now, None);

        assert!(
            store
                .reusable_consent(&key, now, &["childAgents".to_string()])
                .is_none()
        );
    }
}
