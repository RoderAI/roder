//! Controller authorization and pairing-token state for agent-node mode.
//!
//! Controllers are authorized by enrolled certificate fingerprints
//! (mTLS pinning). Pairing tokens exist only to bootstrap enrollment:
//! short-lived, single-use, never accepted in query parameters, and
//! stored hashed with a preview for logs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

const PAIRING_TOKEN_BYTES: usize = 32;
/// Default pairing-token lifetime.
pub const DEFAULT_PAIRING_TTL: time::Duration = time::Duration::minutes(10);

/// Persisted trust state under `<state-dir>/trust.json`.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct TrustState {
    /// Enrolled controller certificate fingerprints (lowercase hex sha256)
    /// with a display label.
    controllers: BTreeMap<String, String>,
    /// Revoked controller fingerprints (kept to refuse re-enrollment).
    revoked: BTreeSet<String>,
}

#[derive(Debug)]
pub struct ControllerTrust {
    path: PathBuf,
    state: Mutex<TrustState>,
}

impl ControllerTrust {
    /// Loads (or initializes) trust state under `state_dir`.
    pub fn open(state_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(state_dir)?;
        let path = state_dir.join("trust.json");
        let state = match std::fs::read(&path) {
            Ok(data) => serde_json::from_slice(&data).unwrap_or_default(),
            Err(_) => TrustState::default(),
        };
        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    pub fn is_trusted(&self, fingerprint: &str) -> bool {
        let state = self.state.lock().unwrap();
        state.controllers.contains_key(fingerprint) && !state.revoked.contains(fingerprint)
    }

    /// Enrolls a controller fingerprint. Revoked fingerprints stay revoked.
    pub fn enroll(&self, fingerprint: &str, label: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            fingerprint.len() == 64 && fingerprint.chars().all(|ch| ch.is_ascii_hexdigit()),
            "controller fingerprint must be a hex sha256"
        );
        let mut state = self.state.lock().unwrap();
        anyhow::ensure!(
            !state.revoked.contains(fingerprint),
            "controller certificate was revoked; issue a new controller identity"
        );
        state
            .controllers
            .insert(fingerprint.to_ascii_lowercase(), label.to_string());
        self.persist(&state)
    }

    pub fn revoke(&self, fingerprint: &str) -> anyhow::Result<bool> {
        let mut state = self.state.lock().unwrap();
        let removed = state.controllers.remove(fingerprint).is_some();
        state.revoked.insert(fingerprint.to_string());
        self.persist(&state)?;
        Ok(removed)
    }

    pub fn controllers(&self) -> Vec<(String, String)> {
        self.state
            .lock()
            .unwrap()
            .controllers
            .iter()
            .map(|(fingerprint, label)| (fingerprint.clone(), label.clone()))
            .collect()
    }

    fn persist(&self, state: &TrustState) -> anyhow::Result<()> {
        std::fs::write(&self.path, serde_json::to_vec_pretty(state)?)?;
        Ok(())
    }
}

struct PairingTokenRecord {
    hash: Vec<u8>,
    preview: String,
    expires_at: OffsetDateTime,
    used: bool,
}

/// In-memory mint/redeem state for single-use pairing tokens.
#[derive(Default)]
pub struct PairingTokens {
    tokens: Mutex<Vec<PairingTokenRecord>>,
}

impl PairingTokens {
    /// Mints a new single-use token; returns `(secret, preview)`. The
    /// secret is shown once to the operator and stored only by hash.
    pub fn mint(&self, ttl: time::Duration) -> (String, String) {
        let bytes: [u8; PAIRING_TOKEN_BYTES] = rand::random();
        let secret = base64_url_no_pad(&bytes);
        let preview = token_preview(&secret);
        self.tokens.lock().unwrap().push(PairingTokenRecord {
            hash: Sha256::digest(secret.as_bytes()).to_vec(),
            preview: preview.clone(),
            expires_at: OffsetDateTime::now_utc() + ttl,
            used: false,
        });
        (secret, preview)
    }

    /// Redeems a token exactly once. Wrong, expired, and reused tokens all
    /// fail closed with coarse reasons (no secret material in errors).
    pub fn redeem(&self, secret: &str) -> anyhow::Result<String> {
        self.redeem_at(secret, OffsetDateTime::now_utc())
    }

    pub fn redeem_at(&self, secret: &str, now: OffsetDateTime) -> anyhow::Result<String> {
        let hash = Sha256::digest(secret.as_bytes());
        let mut tokens = self.tokens.lock().unwrap();
        let Some(record) = tokens
            .iter_mut()
            .find(|record| constant_time_eq(&record.hash, hash.as_slice()))
        else {
            anyhow::bail!("pairing token is not valid");
        };
        anyhow::ensure!(!record.used, "pairing token was already used");
        anyhow::ensure!(now < record.expires_at, "pairing token expired");
        record.used = true;
        Ok(record.preview.clone())
    }
}

pub(crate) fn token_preview(token: &str) -> String {
    let visible: String = token.chars().take(6).collect();
    format!("{visible}…")
}

fn base64_url_no_pad(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (left, right) in a.iter().zip(b.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_tokens_are_single_use_and_expire() {
        let tokens = PairingTokens::default();
        let (secret, preview) = tokens.mint(time::Duration::minutes(5));
        assert!(preview.len() <= 8);

        // Wrong token fails.
        assert!(tokens.redeem("wrong-token").is_err());
        // Valid token redeems once.
        assert_eq!(tokens.redeem(&secret).unwrap(), preview);
        // Reuse fails.
        let reuse = tokens.redeem(&secret).unwrap_err().to_string();
        assert!(reuse.contains("already used"), "{reuse}");

        // Expired tokens fail.
        let (expired, _) = tokens.mint(time::Duration::minutes(5));
        let later = OffsetDateTime::now_utc() + time::Duration::minutes(6);
        let error = tokens.redeem_at(&expired, later).unwrap_err().to_string();
        assert!(error.contains("expired"), "{error}");
    }

    #[test]
    fn controller_trust_enroll_revoke_round_trips() {
        let dir = std::env::temp_dir().join(format!("roder-agent-trust-{}", uuid::Uuid::new_v4()));
        let fingerprint = "a".repeat(64);
        {
            let trust = ControllerTrust::open(&dir).unwrap();
            assert!(!trust.is_trusted(&fingerprint));
            trust.enroll(&fingerprint, "laptop").unwrap();
            assert!(trust.is_trusted(&fingerprint));
        }
        // Trust persists across reopen.
        let trust = ControllerTrust::open(&dir).unwrap();
        assert!(trust.is_trusted(&fingerprint));
        assert!(trust.revoke(&fingerprint).unwrap());
        assert!(!trust.is_trusted(&fingerprint));
        // Revoked fingerprints cannot re-enroll.
        let error = trust.enroll(&fingerprint, "laptop").unwrap_err().to_string();
        assert!(error.contains("revoked"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
