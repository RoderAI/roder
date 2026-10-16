use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    BlaxelClient, BlaxelConfig, BlaxelRunnerSession, CleanupMode, ExpirationPolicy,
    ExpirationPolicyType, SandboxLifecycle,
};

use super::{
    BlaxelForkClient, DeleteReceipt, ForkError, ForkRequest, ForkRuntime, types::validate_name,
};

/// Control-plane readiness. Ready does not prove the browser is safe to bind;
/// the caller must still rotate inherited controls and authorize the child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForkReadiness {
    Missing,
    Pending,
    Ready,
}

/// Local runner settings. Runtime pins and credentials come from the validated
/// child and explicit workspace client, never from the source's externalId.
#[derive(Clone, Debug)]
pub struct ForkSessionOptions {
    pub destination_id: String,
    pub working_dir: String,
    pub cleanup: CleanupMode,
}

// These documents can include application secrets. Never derive Debug or expose
// raw responses through receipts/errors; admin reads are discarded in memory.
#[derive(Deserialize)]
pub(super) struct SandboxDocument {
    metadata: Metadata,
    spec: Value,
    status: String,
    state: Option<String>,
    #[serde(rename = "nodeGeneration")]
    node_generation: Option<String>,
}

#[derive(Deserialize)]
struct Metadata {
    name: String,
    workspace: String,
    url: Option<String>,
}

impl SandboxDocument {
    fn ready(&self) -> Result<bool, ForkError> {
        match self.status.as_str() {
            "DEPLOYED" => Ok(matches!(self.state.as_deref(), Some("RUNNING" | "STANDBY"))),
            "DEPLOYING" | "BUILDING" | "UPLOADING" => Ok(false),
            _ => Err(ForkError::rejected("fork sandbox is not deployable")),
        }
    }

    fn runtime(&self) -> Result<ForkRuntime, ForkError> {
        ForkRuntime::from_sandbox(
            &self.spec,
            self.node_generation.as_deref().unwrap_or_default(),
        )
    }
}

impl BlaxelForkClient {
    async fn sandbox_document(
        &self,
        name: &str,
        show_secrets: bool,
    ) -> Result<Option<SandboxDocument>, ForkError> {
        validate_name(name)?;
        let suffix = if show_secrets {
            "?show_secrets=true"
        } else {
            ""
        };
        let document: Option<SandboxDocument> = self
            .request(
                Method::GET,
                &format!("sandboxes/{}{suffix}", urlencoding::encode(name)),
                None,
            )
            .await?;
        if let Some(document) = &document
            && (document.metadata.name != name || document.metadata.workspace != self.workspace)
        {
            return Err(ForkError::rejected("sandbox lookup identity mismatch"));
        }
        Ok(document)
    }

    /// Capture public configuration pins without requesting environment values.
    /// No wake, process call, update or mutation occurs. Persist these pins with
    /// the checkpoint; source lease/account authorization remains caller-owned.
    pub async fn qualify_source(&self, source: &str) -> Result<ForkRuntime, ForkError> {
        let document = self
            .sandbox_document(source, false)
            .await?
            .ok_or_else(|| ForkError::rejected("fork source is absent"))?;
        if !document.ready()? {
            return Err(ForkError::rejected("fork source is not ready"));
        }
        document.runtime()
    }

    pub(super) async fn check_source(
        &self,
        source: &str,
        expected: &ForkRuntime,
    ) -> Result<(), ForkError> {
        expected.validate()?;
        let actual = self.qualify_source(source).await?;
        if !expected.matches(&actual) {
            return Err(ForkError::rejected("fork source configuration changed"));
        }
        Ok(())
    }

    // Snapshot runtime comes from the snapshot itself; a surviving source may
    // still contribute network/volume settings even after its VM has terminated.
    pub(super) async fn check_snapshot_source_network(
        &self,
        source: &str,
    ) -> Result<(), ForkError> {
        if let Some(document) = self.sandbox_document(source, false).await? {
            super::runtime::validate_attachments(&document.spec)?;
        }
        Ok(())
    }

    async fn owned_child_document(
        &self,
        request: &ForkRequest,
    ) -> Result<Option<SandboxDocument>, ForkError> {
        let Some(document) = self.sandbox_document(request.target(), true).await? else {
            return Ok(None);
        };
        // Even incomplete resources must not be mistaken for our allocation.
        let matches: Vec<_> = document.spec["runtime"]["envs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|env| env["name"] == request.marker_name)
            .collect();
        if matches.len() != 1
            || matches[0]["value"] != request.marker_value.as_str()
            || matches[0].get("secret").is_some_and(|value| value != false)
        {
            return Err(ForkError::rejected("fork generation marker mismatch"));
        }
        Ok(Some(document))
    }

    /// Delete only an exact generation-marker match. Readiness, runtime drift,
    /// expired source authority and browser binding do not block owned cleanup.
    /// Targets must never be reused across allocation generations. An accepted
    /// DELETE still needs later GET absence before recording confirmed cleanup.
    pub async fn delete_child(&self, request: &ForkRequest) -> Result<DeleteReceipt, ForkError> {
        if self.owned_child_document(request).await?.is_none() {
            return Ok(DeleteReceipt::Absent);
        }
        let result: Option<Value> = self
            .request(
                Method::DELETE,
                &format!("sandboxes/{}", urlencoding::encode(request.target())),
                None,
            )
            .await?;
        Ok(if result.is_some() {
            DeleteReceipt::Accepted
        } else {
            DeleteReceipt::Absent
        })
    }

    async fn child_document(
        &self,
        request: &ForkRequest,
        expected: &ForkRuntime,
    ) -> Result<(ForkReadiness, Option<SandboxDocument>), ForkError> {
        expected.validate()?;
        let Some(document) = self.owned_child_document(request).await? else {
            return Ok((ForkReadiness::Missing, None));
        };
        if !document.ready()? {
            return Ok((ForkReadiness::Pending, None));
        }
        if !expected.matches(&document.runtime()?) {
            return Err(ForkError::rejected("fork child configuration mismatch"));
        }
        let policies = document.spec["lifecycle"]["expirationPolicies"]
            .as_array()
            .filter(|policies| policies.len() == 1)
            .ok_or_else(|| ForkError::rejected("fork lifetime policy mismatch"))?;
        let policy = &policies[0];
        let seconds = policy["value"]
            .as_str()
            .and_then(|value| crate::config::duration_seconds(value, "fork max age").ok());
        if policy["type"] != "ttl-max-age"
            || policy["action"] != "delete"
            || seconds != Some(request.max_age.as_secs())
        {
            return Err(ForkError::rejected("fork lifetime policy mismatch"));
        }
        if document
            .metadata
            .url
            .as_deref()
            .is_none_or(|url| url.trim().is_empty())
        {
            return Ok((ForkReadiness::Pending, None));
        }
        self.endpoint(document.metadata.url.as_deref().unwrap_or_default())?;
        Ok((ForkReadiness::Ready, Some(document)))
    }

    /// One bounded read; orchestration owns polling/backoff and cancellation.
    /// Environment values are requested only to verify the non-secret marker.
    pub async fn inspect_child(
        &self,
        request: &ForkRequest,
        expected: &ForkRuntime,
    ) -> Result<ForkReadiness, ForkError> {
        self.child_document(request, expected)
            .await
            .map(|(readiness, _)| readiness)
    }

    /// Re-read and adopt a ready child without create/update/wake/exec/mkdir or
    /// standby lease refresh. None means missing or still pending. The caller
    /// retains its durable allocation handle and must perform browser bind gates.
    /// Generic provider rejoin deliberately rejects these sessions; re-adopt with
    /// the authoritative allocation intent and runtime pins after a restart.
    pub async fn adopt_child(
        &self,
        request: &ForkRequest,
        expected: &ForkRuntime,
        options: &ForkSessionOptions,
    ) -> Result<Option<BlaxelRunnerSession>, ForkError> {
        if options.destination_id.is_empty()
            || !options.working_dir.starts_with('/')
            || options.working_dir.contains('\0')
        {
            return Err(ForkError::rejected("invalid fork session options"));
        }
        let (_, document) = self.child_document(request, expected).await?;
        let Some(document) = document else {
            return Ok(None);
        };
        let endpoint = self.endpoint(document.metadata.url.as_deref().unwrap_or_default())?;
        let config = BlaxelConfig {
            token: self.token.clone(),
            workspace: Some(self.workspace.clone()),
            base_url: self.base_url.clone(),
            sandbox_name: Some(request.target().into()),
            external_id: None,
            sandbox_name_prefix: "fork".into(),
            image: expected.image.clone(),
            memory_mb: expected.memory_mb,
            ports: expected.ports.clone(),
            region: Some(expected.region.clone()),
            egress_gateway: None,
            egress_workspace: None,
            ttl: None,
            standby_after_seconds: None,
            lifecycle: Some(SandboxLifecycle {
                expiration_policies: vec![ExpirationPolicy {
                    kind: ExpirationPolicyType::TtlMaxAge,
                    value: format!("{}s", request.max_age.as_secs()),
                }],
            }),
            working_dir: options.working_dir.clone(),
            cleanup: options.cleanup,
        };
        Ok(Some(
            BlaxelRunnerSession::new(
                BlaxelClient::new(self.http.clone(), &config),
                &config,
                options.destination_id.clone(),
                request.target().into(),
                None,
                endpoint,
                document.state.as_deref() == Some("STANDBY"),
            )
            .with_fork_allocation(ForkAllocation {
                client: self.clone(),
                request: request.clone(),
            }),
        ))
    }

    fn endpoint(&self, endpoint: &str) -> Result<String, ForkError> {
        crate::workspace::validate_endpoint(&self.base_url, endpoint)
            .map_err(|_| ForkError::rejected("unsupported fork endpoint"))?;
        Ok(endpoint.trim_end_matches('/').into())
    }
}

/// Session-local cleanup ownership. Durable allocation intent lives in the
/// caller; this context is never serialized or used by generic rejoin.
pub(crate) struct ForkAllocation {
    pub(crate) client: BlaxelForkClient,
    pub(crate) request: ForkRequest,
}
