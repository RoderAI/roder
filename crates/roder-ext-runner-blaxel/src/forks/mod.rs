//! Explicit snapshot/fork operations for an already-authorized allocation.
//!
//! These primitives do not enable the generic runner snapshot capability, retry
//! mutations, or authorize inherited browser controls. Callers own durable intents,
//! capacity admission, ownership checks, cancellation and cleanup reconciliation.

mod adoption;
mod http;
mod runtime;
mod snapshot_spec;
mod types;

use std::time::Duration;

use reqwest::Method;
use serde_json::{Value, json};

use crate::Redacted;

pub(crate) use adoption::ForkAllocation;
pub use adoption::{ForkReadiness, ForkSessionOptions};
pub use runtime::ForkRuntime;
pub use snapshot_spec::SnapshotSpec;

pub use types::{
    DeleteReceipt, ForkError, ForkReceipt, ForkRequest, Outcome, Snapshot, SnapshotSource,
};
use types::{validate_id, validate_name};

/// An explicit workspace profile, independent of the default runner credential.
/// It does not read environment variables or follow redirects. Clone this client
/// to share connection pooling within one workspace, never across profiles.
#[derive(Clone)]
pub struct BlaxelForkClient {
    http: reqwest::Client,
    token: Redacted,
    workspace: String,
    base_url: String,
}

impl BlaxelForkClient {
    pub fn new(token: Redacted, workspace: impl Into<String>) -> Result<Self, ForkError> {
        Self::with_base_url(token, workspace, crate::config::DEFAULT_BASE_URL)
    }

    /// Alternate endpoints are limited to loopback for isolated transport tests.
    pub fn with_base_url(
        token: Redacted,
        workspace: impl Into<String>,
        base_url: &str,
    ) -> Result<Self, ForkError> {
        let profile = crate::BlaxelWorkspace::with_base_url(token, workspace, base_url)
            .map_err(|_| ForkError::rejected("invalid workspace profile"))?;
        let http = reqwest::Client::builder()
            .user_agent(crate::USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| ForkError::rejected("cannot construct provider client"))?;
        Ok(Self {
            http,
            token: profile.token,
            workspace: profile.workspace,
            base_url: profile.base_url,
        })
    }

    /// Recheck the source against persisted public-network/runtime pins, then
    /// capture it. Persist (workspace, source, name) before dispatch. A timeout, 409 or lost
    /// response requires inspecting that exact identity before any retry.
    pub async fn create_snapshot(
        &self,
        source: &str,
        name: &str,
        expected: &ForkRuntime,
    ) -> Result<Snapshot, ForkError> {
        validate_name(source)?;
        validate_name(name)?;
        self.check_source(source, expected).await?;
        let snapshot: Snapshot = self
            .request(
                Method::POST,
                "snapshots",
                Some(json!({"name":name,"source":{"name":source}})),
            )
            .await?
            .ok_or_else(|| ForkError::unknown("missing snapshot receipt"))?;
        if !snapshot.matches(&self.workspace, source, name) || validate_id(&snapshot.id).is_err() {
            return Err(ForkError::unknown("snapshot receipt identity mismatch"));
        }
        Ok(snapshot)
    }

    pub async fn get_snapshot(&self, id: &str) -> Result<Option<Snapshot>, ForkError> {
        validate_id(id)?;
        let snapshot: Option<Snapshot> = self
            .request(
                Method::GET,
                &format!("snapshots/{}", urlencoding::encode(id)),
                None,
            )
            .await?;
        if let Some(snapshot) = &snapshot
            && (snapshot.id != id
                || snapshot.workspace != self.workspace
                || snapshot.source.kind != "sandbox")
        {
            return Err(ForkError::rejected("snapshot lookup identity mismatch"));
        }
        Ok(snapshot)
    }

    /// Search the workspace catalog, including snapshots whose source is gone.
    /// The caller must also validate its durable generation/ownership record.
    pub async fn find_snapshot(
        &self,
        source: &str,
        name: &str,
    ) -> Result<Option<Snapshot>, ForkError> {
        validate_name(source)?;
        validate_name(name)?;
        let mut cursor: Option<String> = None;
        let mut found = None;
        // Bound both memory and requests even if a provider repeats a cursor.
        for _ in 0..100 {
            let mut path = "snapshots?limit=100".to_string();
            if let Some(cursor) = &cursor {
                path.push_str(&format!("&cursor={}", urlencoding::encode(cursor)));
            }
            #[derive(serde::Deserialize)]
            struct Page {
                data: Vec<Snapshot>,
                meta: Meta,
            }
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Meta {
                has_more: bool,
                next_cursor: Option<String>,
            }
            let page: Page = self
                .request(Method::GET, &path, None)
                .await?
                .ok_or_else(|| ForkError::rejected("snapshot catalog unavailable"))?;
            for snapshot in page.data {
                if snapshot.workspace != self.workspace {
                    return Err(ForkError::rejected("snapshot catalog workspace mismatch"));
                }
                if snapshot.name == name && snapshot.source.name == source {
                    if !snapshot.matches(&self.workspace, source, name)
                        || validate_id(&snapshot.id).is_err()
                        || found.is_some()
                    {
                        return Err(ForkError::rejected("snapshot catalog identity mismatch"));
                    }
                    found = Some(snapshot);
                }
            }
            if !page.meta.has_more {
                return Ok(found);
            }
            let next = page
                .meta
                .next_cursor
                .filter(|next| !next.is_empty() && next.len() <= 4096)
                .ok_or_else(|| ForkError::rejected("invalid snapshot catalog cursor"))?;
            if cursor.as_ref() == Some(&next) {
                return Err(ForkError::rejected("repeated snapshot catalog cursor"));
            }
            cursor = Some(next);
        }
        Err(ForkError::rejected("snapshot catalog page limit exceeded"))
    }

    /// Delete only a previously ownership-validated ID. Acceptance still requires
    /// a subsequent GET returning None before recording confirmed deletion.
    pub async fn delete_snapshot(&self, id: &str) -> Result<DeleteReceipt, ForkError> {
        validate_id(id)?;
        let result: Option<Value> = self
            .request(
                Method::DELETE,
                &format!("snapshots/{}", urlencoding::encode(id)),
                None,
            )
            .await?;
        Ok(if result.is_some() {
            DeleteReceipt::Accepted
        } else {
            DeleteReceipt::Absent
        })
    }

    /// Recheck source runtime pins, then fork live state without a persistent snapshot.
    pub async fn fork_live(
        &self,
        source: &str,
        request: &ForkRequest,
        expected: &ForkRuntime,
    ) -> Result<ForkReceipt, ForkError> {
        validate_name(source)?;
        if source == request.target() {
            return Err(ForkError::rejected("fork target equals source"));
        }
        self.check_source(source, expected).await?;
        self.fork(
            &format!("sandboxes/{}/fork", urlencoding::encode(source)),
            request,
            None,
        )
        .await
    }

    /// Re-read snapshot readiness/configuration and any surviving source's
    /// attachment settings before forking. Expected pins must originate from the
    /// authorized checkpoint capture, not an arbitrary imported snapshot.
    /// A ready receipt does not replace application authorization.
    pub async fn fork_snapshot(
        &self,
        snapshot: &Snapshot,
        request: &ForkRequest,
        expected: &ForkRuntime,
    ) -> Result<ForkReceipt, ForkError> {
        validate_id(&snapshot.id)?;
        expected.validate()?;
        if snapshot.workspace != self.workspace
            || snapshot.source.kind != "sandbox"
            || !snapshot.is_ready()
            || snapshot.source.name == request.target()
        {
            return Err(ForkError::rejected("snapshot is not eligible for fork"));
        }
        let current = self
            .get_snapshot(&snapshot.id)
            .await?
            .ok_or_else(|| ForkError::rejected("fork snapshot is absent"))?;
        if !current.is_ready()
            || current.name != snapshot.name
            || current.source.name != snapshot.source.name
            || !current
                .spec
                .as_ref()
                .is_some_and(|spec| spec.matches(expected))
        {
            return Err(ForkError::rejected(
                "snapshot configuration or readiness changed",
            ));
        }
        if !current.source.deleted {
            self.check_snapshot_source_network(&current.source.name)
                .await?;
        }
        self.fork(
            &format!("snapshots/{}/fork", urlencoding::encode(&snapshot.id)),
            request,
            Some(&snapshot.id),
        )
        .await
    }

    async fn fork(
        &self,
        path: &str,
        request: &ForkRequest,
        snapshot_id: Option<&str>,
    ) -> Result<ForkReceipt, ForkError> {
        let receipt: ForkReceipt = self
            .request(Method::POST, path, Some(request.body()))
            .await?
            .ok_or_else(|| ForkError::unknown("missing fork receipt"))?;
        let actual_snapshot = receipt.snapshot_id.as_deref().filter(|id| !id.is_empty());
        if receipt.name != request.target()
            || receipt.target_type != "sandbox"
            || actual_snapshot != snapshot_id
        {
            let mut error = ForkError::unknown("fork receipt identity mismatch");
            error.fork_receipt = Some(receipt);
            return Err(error);
        }
        Ok(receipt)
    }
}
