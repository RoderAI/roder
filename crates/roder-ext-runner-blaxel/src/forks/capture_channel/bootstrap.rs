//! Source-only neutral startup. The caller owns admission and durable dispatch.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::signature::{ED25519, UnparsedPublicKey};
use std::time::{SystemTime, UNIX_EPOCH};

const PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];
const MAX_SAFE: u64 = 9_007_199_254_740_991;

/// Expected application identities after definitive capture and source resume
/// admission. The public key must come from the retained capture preparation.
pub struct SourceApiBootstrapBinding<'a> {
    /// Application allocation ID, distinct from execution ID or provider name.
    pub source_id: &'a str,
    /// The definitively settled checkpoint covered by source resume admission.
    pub checkpoint_id: &'a str,
    /// The exact epoch of that recorded admission.
    pub admission_epoch: u32,
    /// Original caller/root expiry in Unix milliseconds; never a fresh budget.
    pub max_expires_at_ms: u64,
}

/// A verified, bounded signed command, not browser activation or fork authority.
/// No Clone/Debug/Serde. Retain requested state before obtaining/consuming this;
/// recovery must query bootstrap status instead of constructing another permit.
pub struct SourceApiBootstrapPermit {
    pin: CaptureChannelPin,
    wire: Value,
    deadline: Instant,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ticket {
    schema_version: u8,
    body: String,
    signature: String,
}

fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn decode(value: &str, max: usize) -> Result<Vec<u8>, ForkError> {
    if value.len() > max {
        return Err(refused());
    }
    let bytes = STANDARD.decode(value).map_err(|_| refused())?;
    if STANDARD.encode(&bytes) != value {
        return Err(refused());
    }
    Ok(bytes)
}
fn refused() -> ForkError {
    ForkError::rejected("source API bootstrap binding unavailable")
}
fn now_ms() -> Result<u64, ForkError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .ok_or_else(refused)
}

impl CaptureChannel {
    /// Authenticate a source-purpose issuer ticket locally, before any bytes
    /// reach the VM parser. This performs no network operation or admission.
    pub fn prepare_source_api_bootstrap(
        &self,
        ticket: Value,
        public_key: &str,
        binding: &SourceApiBootstrapBinding<'_>,
        deadline: Instant,
    ) -> Result<SourceApiBootstrapPermit, ForkError> {
        let now = now_ms()?;
        if Instant::now() >= deadline.min(self.deadline)
            || !id(binding.source_id)
            || !id(binding.checkpoint_id)
            || binding.admission_epoch > i32::MAX as u32
            || binding.max_expires_at_ms > MAX_SAFE
            || binding.max_expires_at_ms <= now
        {
            return Err(refused());
        }
        let key = decode(public_key, 60)?;
        if key.len() != 44 || !key.starts_with(&PREFIX) {
            return Err(refused());
        }
        let parsed: Ticket = serde_json::from_value(ticket.clone()).map_err(|_| refused())?;
        if parsed.schema_version != 1 {
            return Err(refused());
        }
        let body = decode(&parsed.body, 1368)?;
        let signature = decode(&parsed.signature, 88)?;
        if body.len() > 1024 || signature.len() != 64 {
            return Err(refused());
        }
        UnparsedPublicKey::new(&ED25519, &key[12..])
            .verify(&body, &signature)
            .map_err(|_| refused())?;
        let value: Value = serde_json::from_slice(&body).map_err(|_| refused())?;
        if serde_json::to_vec(&value).map_err(|_| refused())? != body {
            return Err(refused());
        }
        let issued = value["issued_at_ms"].as_u64().ok_or_else(refused)?;
        let expires = value["expires_at_ms"].as_u64().ok_or_else(refused)?;
        let nonce = value["nonce"].as_str().ok_or_else(refused)?;
        if issued == 0
            || issued > now
            || expires <= now
            || expires <= issued
            || expires - issued > 15000
            || expires > binding.max_expires_at_ms
            || nonce.len() != 32
            || !nonce
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || value
                != json!({"schema_version":1,"operation":"start-neutral-api","purpose":"source",
                "source_id":binding.source_id,"target_id":binding.source_id,"checkpoint_id":binding.checkpoint_id,
                "request_digest":self.pin.request_digest,"admission_epoch":binding.admission_epoch,
                "issued_at_ms":issued,"expires_at_ms":expires,"nonce":nonce})
        {
            return Err(refused());
        }
        let wire = json!({"operation":"bootstrap-api","ticket":ticket});
        if serde_json::to_vec(&wire).map_err(|_| refused())?.len() > 2048 {
            return Err(refused());
        }
        Ok(SourceApiBootstrapPermit {
            pin: self.pin.clone(),
            wire,
            deadline: deadline
                .min(self.deadline)
                .min(Instant::now() + Duration::from_millis(expires - now)),
        })
    }

    /// Consume one signed delivery opportunity. There is no retry, redirect,
    /// account credential or cookie. A lost/malformed response is Unknown; use
    /// `bootstrap_status` against the retained digest, never issue another POST.
    pub async fn bootstrap_source_api(
        &self,
        permit: SourceApiBootstrapPermit,
    ) -> Result<(), ForkError> {
        let SourceApiBootstrapPermit {
            pin,
            wire,
            deadline,
        } = permit;
        if pin != self.pin || Instant::now() >= deadline.min(self.deadline) {
            return Err(refused());
        }
        let unknown = || ForkError::unknown("source API bootstrap delivery unconfirmed");
        timeout_at(deadline.min(self.deadline), async {
            let mut response = self
                .http
                .post(format!("{}/bootstrap-api", self.pin.origin))
                .header(reqwest::header::ACCEPT, "application/json")
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .json(&wire)
                .send()
                .await
                .map_err(|_| unknown())?;
            if response.status() != reqwest::StatusCode::OK
                || [
                    reqwest::header::CONTENT_TYPE,
                    reqwest::header::CACHE_CONTROL,
                ]
                .iter()
                .any(|name| response.headers().get_all(name).iter().count() != 1)
                || response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    != Some("application/json")
                || response
                    .headers()
                    .get(reqwest::header::CACHE_CONTROL)
                    .and_then(|v| v.to_str().ok())
                    != Some("no-store")
                || response
                    .headers()
                    .contains_key(reqwest::header::CONTENT_ENCODING)
            {
                return Err(unknown());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| unknown())? {
                if bytes.len().saturating_add(chunk.len()) > 1024 {
                    return Err(unknown());
                }
                bytes.extend_from_slice(&chunk);
            }
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| unknown())?;
            if serde_json::to_vec(&value).map_err(|_| unknown())? != bytes
                || value != json!({"ok":true,"neutral_api_started":true,"full_vm_seal":false})
                || Instant::now() >= deadline.min(self.deadline)
            {
                return Err(unknown());
            }
            Ok(())
        })
        .await
        .map_err(|_| unknown())?
    }
}

#[cfg(test)]
#[path = "bootstrap/tests.rs"]
mod tests;
