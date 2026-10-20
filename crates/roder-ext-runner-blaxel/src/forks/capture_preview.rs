//! Owned capture preview lifecycle. The caller retains intent and requested phase
//! before requesting a single create permit; recovery never reissues that permit.
use std::future::Future;

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout_at};

use super::capture_channel::{PORT, preview_origin, validate_digest};
use super::{
    BlaxelForkClient, CaptureChannel, CaptureChannelPin, DeleteReceipt, ForkError, ForkRuntime,
    Outcome,
};

/// Public resource identity, not create authority. Retain this and the requested
/// phase durably before preparation; never prepare again after dispatch/unknown.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapturePreviewIntent {
    workspace: String,
    source: String,
    external_id: String,
    runtime: ForkRuntime,
    request_digest: String,
}

/// One process-local create opportunity after an absence/source check. It cannot
/// be cloned or recovered from JSON. Backend first-ACK authority remains required.
pub struct CapturePreviewCreatePermit {
    intent: CapturePreviewIntent,
    control_base: String,
    deadline: Instant,
}

impl CapturePreviewIntent {
    /// Deterministic, operation-specific provider identity for retention/cleanup.
    pub fn preview_name(&self) -> String {
        format!(
            "verify-capture-{}",
            self.request_digest.get(..32).unwrap_or_default()
        )
    }

    fn check(&self, client: &BlaxelForkClient) -> Result<(), ForkError> {
        super::types::validate_name(&self.source)?;
        validate_digest(&self.request_digest)?;
        self.runtime.validate()?;
        if self.workspace != client.workspace
            || self.external_id.is_empty()
            || self.external_id.len() > 256
            || self.external_id.chars().any(char::is_control)
            || !self
                .runtime
                .ports
                .iter()
                .any(|p| p.target == PORT && p.protocol == crate::PortProtocol::Http)
        {
            return Err(ForkError::rejected("capture preview intent unavailable"));
        }
        Ok(())
    }

    fn collection(&self) -> String {
        format!("sandboxes/{}/previews", urlencoding::encode(&self.source))
    }
    fn path(&self) -> String {
        format!("{}/{}", self.collection(), self.preview_name())
    }

    fn pin(&self, value: &Value) -> Result<CaptureChannelPin, ForkError> {
        let metadata = &value["metadata"];
        let spec = &value["spec"];
        if metadata["name"] != self.preview_name()
            || spec["port"] != PORT
            || spec["public"] != true
            || !metadata["workspace"].is_null() && metadata["workspace"] != self.workspace
            || !metadata["resourceName"].is_null() && metadata["resourceName"] != self.source
            || !metadata["resourceType"].is_null()
                && !metadata["resourceType"].as_str().is_some_and(|v| {
                    ["sandbox", "sandboxes"].contains(&v.to_ascii_lowercase().as_str())
                })
            || ["customDomain", "prefixUrl"]
                .iter()
                .any(|key| !spec[key].is_null() && spec[key] != "")
            || ["requestHeaders", "responseHeaders"].iter().any(|key| {
                !spec[key].is_null() && !spec[key].as_object().is_some_and(|v| v.is_empty())
            })
            || spec["ttl"] != "10m"
            || !value["status"].is_null() && value["status"] != "DEPLOYED"
        {
            return Err(ForkError::rejected("capture preview configuration changed"));
        }
        let origin = preview_origin(
            spec["url"]
                .as_str()
                .ok_or_else(|| ForkError::rejected("capture preview URL unavailable"))?,
        )?;
        Ok(CaptureChannelPin {
            workspace: self.workspace.clone(),
            source: self.source.clone(),
            external_id: self.external_id.clone(),
            origin,
            preview_name: self.preview_name(),
            request_digest: self.request_digest.clone(),
        })
    }
}

impl BlaxelForkClient {
    /// Build only retention data. No provider request or grant occurs here.
    pub fn capture_preview_intent(
        &self,
        source: &str,
        external_id: &str,
        runtime: &ForkRuntime,
        request_digest: &str,
    ) -> Result<CapturePreviewIntent, ForkError> {
        let intent = CapturePreviewIntent {
            workspace: self.workspace.clone(),
            source: source.into(),
            external_id: external_id.into(),
            runtime: runtime.clone(),
            request_digest: request_digest.into(),
        };
        intent.check(self)?;
        Ok(intent)
    }

    async fn capture_preview_source(
        &self,
        intent: &CapturePreviewIntent,
        running: bool,
    ) -> Result<(), ForkError> {
        intent.check(self)?;
        let source = self
            .observe_original_source(&intent.source, &intent.external_id)
            .await?;
        if !source.runtime().matches(&intent.runtime) || running && !source.is_running() {
            return Err(ForkError::rejected("capture preview source changed"));
        }
        Ok(())
    }

    /// After durable requested-phase retention, check absence without overwriting
    /// any preview. A lost response after create is reconciled, never re-prepared.
    pub async fn prepare_capture_preview(
        &self,
        intent: &CapturePreviewIntent,
        deadline: Instant,
    ) -> Result<CapturePreviewCreatePermit, ForkError> {
        bounded(deadline, false, async {
            self.capture_preview_source(intent, true).await?;
            if self
                .request::<Value>(Method::GET, &intent.path(), None)
                .await?
                .is_some()
            {
                return Err(ForkError::rejected("capture preview already exists"));
            }
            Ok(CapturePreviewCreatePermit {
                intent: intent.clone(),
                control_base: self.base_url.clone(),
                deadline,
            })
        })
        .await
    }

    /// Consume the permit before one POST with no force/update/retry. Persist the
    /// returned pin; an unknown outcome requires GET of this exact retained name.
    pub async fn create_capture_preview(
        &self,
        permit: CapturePreviewCreatePermit,
    ) -> Result<CaptureChannelPin, ForkError> {
        let CapturePreviewCreatePermit {
            intent,
            control_base,
            deadline,
        } = permit;
        if control_base != self.base_url {
            return Err(ForkError::rejected("capture preview account changed"));
        }
        bounded(deadline, false, self.capture_preview_source(&intent, true)).await?;
        bounded(deadline, true, async {
            let result: Option<Value> = self.request(Method::POST, &intent.collection(), Some(json!({
                "metadata":{"name":intent.preview_name()},"spec":{"port":PORT,"public":true,"ttl":"10m","requestHeaders":{},"responseHeaders":{}}
            }))).await.map_err(|mut error| { if error.status == Some(409) { error.outcome = Outcome::Unknown; } error })?;
            let value = result.ok_or_else(|| ForkError::unknown("capture preview create unconfirmed"))?;
            intent.pin(&value).map_err(|_| ForkError::unknown("capture preview create unconfirmed"))
        }).await
    }

    /// Read-only settlement of one retained create/delete identity. Absence is
    /// not permission to repeat creation. This read never wakes a standby VM.
    pub async fn reconcile_capture_preview(
        &self,
        intent: &CapturePreviewIntent,
        deadline: Instant,
    ) -> Result<Option<CaptureChannelPin>, ForkError> {
        bounded(deadline, false, async {
            self.capture_preview_source(intent, false).await?;
            self.request::<Value>(Method::GET, &intent.path(), None)
                .await?
                .as_ref()
                .map(|v| intent.pin(v))
                .transpose()
        })
        .await
    }

    /// Prove only route absence by its exact retained identity, even after the
    /// original VM is gone. A 404 is never authority to recreate the route.
    pub async fn capture_preview_absent(
        &self,
        intent: &CapturePreviewIntent,
        deadline: Instant,
    ) -> Result<bool, ForkError> {
        bounded(deadline, false, async {
            intent.check(self)?;
            Ok(self
                .request::<Value>(Method::GET, &intent.path(), None)
                .await?
                .is_none())
        })
        .await
    }

    /// Delete only an unchanged owned route, independently of the stopped Go API.
    /// Accepted deletion requires another reconcile read to prove absence.
    pub async fn delete_capture_preview(
        &self,
        intent: &CapturePreviewIntent,
        pin: &CaptureChannelPin,
        deadline: Instant,
    ) -> Result<DeleteReceipt, ForkError> {
        bounded(deadline, true, async {
            self.capture_preview_source(intent, false).await?;
            let Some(value) = self
                .request::<Value>(Method::GET, &intent.path(), None)
                .await?
            else {
                return Ok(DeleteReceipt::Absent);
            };
            if intent.pin(&value)? != *pin {
                return Err(ForkError::rejected("capture preview route changed"));
            }
            self.request::<Value>(Method::DELETE, &intent.path(), None)
                .await?;
            Ok(DeleteReceipt::Accepted)
        })
        .await
    }

    /// Observe only a running original VM and its operation-specific public route.
    /// This grants no preparation, bootstrap or full-VM capture authority.
    pub async fn observe_capture_channel(
        &self,
        source: &str,
        external_id: &str,
        expected: &ForkRuntime,
        request_digest: &str,
        deadline: Instant,
    ) -> Result<CaptureChannel, ForkError> {
        let intent = self.capture_preview_intent(source, external_id, expected, request_digest)?;
        bounded(deadline, false, async {
            self.capture_preview_source(&intent, true).await?;
            let value = self
                .request::<Value>(Method::GET, &intent.path(), None)
                .await?
                .ok_or_else(|| ForkError::rejected("capture preview is absent"))?;
            CaptureChannel::new(intent.pin(&value)?, deadline)
        })
        .await
    }

    /// Fresh source/runtime/route reads must match the retained pin exactly.
    pub async fn recover_capture_channel(
        &self,
        pin: &CaptureChannelPin,
        expected: &ForkRuntime,
        deadline: Instant,
    ) -> Result<CaptureChannel, ForkError> {
        if pin.workspace != self.workspace {
            return Err(ForkError::rejected("capture channel workspace changed"));
        }
        let channel = self
            .observe_capture_channel(
                &pin.source,
                &pin.external_id,
                expected,
                &pin.request_digest,
                deadline,
            )
            .await?;
        if channel.pin() != pin {
            return Err(ForkError::rejected("capture channel route changed"));
        }
        Ok(channel)
    }
}

async fn bounded<T>(
    deadline: Instant,
    mutation: bool,
    work: impl Future<Output = Result<T, ForkError>>,
) -> Result<T, ForkError> {
    if Instant::now() >= deadline {
        return Err(ForkError::rejected("capture preview deadline expired"));
    }
    timeout_at(deadline, work).await.map_err(|_| {
        if mutation {
            ForkError::unknown("capture preview mutation unconfirmed")
        } else {
            ForkError::rejected("capture preview observation expired")
        }
    })?
}
