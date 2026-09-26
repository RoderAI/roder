//! Persisted browser-pairing credentials.
//!
//! The browser extension stores one endpoint + bearer token and reconnects to it
//! on its own. If Roder minted a fresh token and took a fresh OS-assigned port on
//! every start, that stored pairing would be dead the moment Roder restarted and
//! the user would have to walk the `/pair` flow again — so the pairing that the
//! extension holds is kept in `<config-dir>/remote-pairing.json` and reused.
//!
//! The file holds a bearer token, so it is written with owner-only permissions
//! and never logged; callers print the usual token preview instead.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{RemoteToken, generate_remote_token_from_os};

const PAIRING_FILE: &str = "remote-pairing.json";

/// The endpoint + token a previously paired browser extension is still holding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedPairing {
    /// Bearer token the extension authenticates with.
    pub token: String,
    /// Loopback port the extension reconnects to. `0` means "not yet bound".
    #[serde(default)]
    pub port: u16,
}

fn pairing_path() -> PathBuf {
    roder_config::config_dir().join(PAIRING_FILE)
}

fn read_from(path: &Path) -> Option<PersistedPairing> {
    let text = std::fs::read_to_string(path).ok()?;
    let pairing: PersistedPairing = serde_json::from_str(&text).ok()?;
    if pairing.token.is_empty() {
        return None;
    }
    Some(pairing)
}

fn write_to(path: &Path, pairing: &PersistedPairing) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(pairing)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The file holds a bearer token; keep it owner-only.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// True when a browser has been paired before, i.e. a pairing file exists.
/// Callers use this to decide whether to bring the listener back up on startup
/// without creating a pairing for a user who never asked for one.
pub fn exists() -> bool {
    read_from(&pairing_path()).is_some()
}

/// Load the stored pairing, creating one on first use.
pub fn load_or_create() -> anyhow::Result<PersistedPairing> {
    load_or_create_at(&pairing_path())
}

pub fn load_or_create_at(path: &Path) -> anyhow::Result<PersistedPairing> {
    if let Some(existing) = read_from(path) {
        return Ok(existing);
    }
    let pairing = PersistedPairing {
        token: generate_remote_token_from_os()?.secret().to_string(),
        port: 0,
    };
    write_to(path, &pairing)?;
    Ok(pairing)
}

/// Record the port the listener actually bound, so the next start reuses it.
pub fn remember_port(port: u16) -> anyhow::Result<()> {
    remember_port_at(&pairing_path(), port)
}

pub fn remember_port_at(path: &Path, port: u16) -> anyhow::Result<()> {
    let mut pairing = load_or_create_at(path)?;
    if pairing.port == port {
        return Ok(());
    }
    pairing.port = port;
    write_to(path, &pairing)
}

/// Mint a new token and forget the old pairing, invalidating every paired
/// browser. This is the explicit "unpair everything" path.
pub fn rotate() -> anyhow::Result<PersistedPairing> {
    rotate_at(&pairing_path())
}

pub fn rotate_at(path: &Path) -> anyhow::Result<PersistedPairing> {
    let pairing = PersistedPairing {
        token: generate_remote_token_from_os()?.secret().to_string(),
        port: 0,
    };
    write_to(path, &pairing)?;
    Ok(pairing)
}

/// The stored pairing as a [`RemoteToken`].
pub fn token_of(pairing: &PersistedPairing) -> anyhow::Result<RemoteToken> {
    RemoteToken::new(pairing.token.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_is_stable_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PAIRING_FILE);
        let first = load_or_create_at(&path).unwrap();
        let second = load_or_create_at(&path).unwrap();
        assert_eq!(first.token, second.token, "token must survive a restart");
        assert!(!first.token.is_empty());
    }

    #[test]
    fn remembered_port_is_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PAIRING_FILE);
        load_or_create_at(&path).unwrap();
        remember_port_at(&path, 51234).unwrap();
        assert_eq!(load_or_create_at(&path).unwrap().port, 51234);
    }

    #[test]
    fn rotate_replaces_the_token_and_clears_the_port() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PAIRING_FILE);
        let before = load_or_create_at(&path).unwrap();
        remember_port_at(&path, 4321).unwrap();
        let after = rotate_at(&path).unwrap();
        assert_ne!(before.token, after.token);
        assert_eq!(after.port, 0);
    }

    #[cfg(unix)]
    #[test]
    fn pairing_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PAIRING_FILE);
        load_or_create_at(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
