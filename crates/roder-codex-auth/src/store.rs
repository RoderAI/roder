use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::{Tokens, normalize};

const PRIMARY_FILE: &str = "codex.json";
const BACKUP_FILE: &str = "codex.json.bak";
const AUDIT_FILE: &str = "codex.audit.jsonl";

pub struct Store {
    data_dir: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Self {
        Self {
            data_dir: roder_data_dir(),
        }
    }

    pub fn load(&self) -> anyhow::Result<Tokens> {
        let path = self.path();
        match load_tokens_from(&path) {
            Ok(tokens) if has_any_token(&tokens) => Ok(tokens),
            Ok(_) => self.restore_from_backup("primary_empty"),
            Err(err) => self.restore_from_backup(&format!("primary_error:{err}")),
        }
    }

    pub fn save(&self, mut tokens: Tokens) -> anyhow::Result<()> {
        normalize(&mut tokens);
        let path = self.path();
        let backup_path = self.backup_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_tokens(&path, &tokens)?;
        write_tokens(&backup_path, &tokens)?;
        self.audit("save", None)?;
        Ok(())
    }

    pub fn delete(&self) -> anyhow::Result<()> {
        let path = self.path();
        let backup_path = self.backup_path();
        remove_if_exists(&path)?;
        remove_if_exists(&backup_path)?;
        self.audit("delete", None)?;
        Ok(())
    }

    fn restore_from_backup(&self, reason: &str) -> anyhow::Result<Tokens> {
        let backup_path = self.backup_path();
        let tokens = load_tokens_from(&backup_path)?;
        if !has_any_token(&tokens) {
            return Ok(Tokens::default());
        }
        write_tokens(&self.path(), &tokens)?;
        self.audit("restore_backup", Some(reason))?;
        Ok(tokens)
    }

    fn path(&self) -> PathBuf {
        self.auth_dir().join(PRIMARY_FILE)
    }

    fn backup_path(&self) -> PathBuf {
        self.auth_dir().join(BACKUP_FILE)
    }

    fn auth_dir(&self) -> PathBuf {
        self.data_dir.join("auth")
    }

    fn audit(&self, action: &str, reason: Option<&str>) -> anyhow::Result<()> {
        let audit_path = self.auth_dir().join(AUDIT_FILE);
        if let Some(parent) = audit_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let entry = json!({
            "timestamp_ms": now_millis(),
            "action": action,
            "reason": reason,
            "pid": std::process::id(),
            "cwd": std::env::current_dir().ok().map(|path| path.display().to_string()),
            "exe": std::env::current_exe().ok().map(|path| path.display().to_string()),
        });
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(audit_path)?;
        writeln!(file, "{entry}")?;
        Ok(())
    }

    #[cfg(test)]
    fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
}

fn load_tokens_from(path: &Path) -> anyhow::Result<Tokens> {
    match fs::read_to_string(path) {
        Ok(contents) if contents.trim().is_empty() => Ok(Tokens::default()),
        Ok(contents) => {
            let mut tokens: Tokens = serde_json::from_str(&contents)?;
            normalize(&mut tokens);
            Ok(tokens)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Tokens::default()),
        Err(err) => Err(err.into()),
    }
}

fn write_tokens(path: &Path, tokens: &Tokens) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec_pretty(tokens)?;
    let tmp_path = temp_path_for(path);
    fs::write(&tmp_path, [data, b"\n".to_vec()].concat())?;
    restrict_permissions(&tmp_path)?;
    fs::rename(&tmp_path, path)?;
    restrict_permissions(path)?;
    Ok(())
}

fn remove_if_exists(path: &Path) -> anyhow::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn temp_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("codex.json");
    path.with_file_name(format!("{file_name}.tmp-{}", std::process::id()))
}

fn restrict_permissions(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn roder_data_dir() -> PathBuf {
    std::env::var_os("RODER_DATA_DIR")
        .or_else(|| std::env::var_os("RODER_CONFIG_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".roder")
        })
}

fn has_any_token(tokens: &Tokens) -> bool {
    !tokens.refresh.trim().is_empty() || !tokens.access.trim().is_empty()
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0))
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn store_loads_roder_owned_tokens() {
        let home = unique_test_home("roder-owned-store");
        std::fs::create_dir_all(home.join(".roder").join("auth")).unwrap();
        std::fs::write(
            home.join(".roder").join("auth").join("codex.json"),
            r#"{
              "type": "oauth",
              "access": " roder-access ",
              "refresh": " roder-refresh ",
              "account_id": " acct_roder ",
              "expires": 123
            }"#,
        )
        .unwrap();

        let loaded = Store::with_data_dir(home.join(".roder")).load().unwrap();

        assert_eq!(loaded.access, "roder-access");
        assert_eq!(loaded.refresh, "roder-refresh");
        assert_eq!(loaded.account_id, "acct_roder");
        assert_eq!(loaded.expires, 123);
    }

    #[test]
    fn store_load_ignores_codex_cli_tokens_when_roder_store_is_missing() {
        let home = unique_test_home("codex-cli-ignored");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(
            home.join(".codex").join("auth.json"),
            r#"{
              "auth_mode": "chatgpt",
              "OPENAI_API_KEY": null,
              "tokens": {
                "access_token": " codex-access ",
                "refresh_token": " codex-refresh ",
                "account_id": " acct_codex "
              },
              "last_refresh": "2026-05-18T12:00:00Z"
            }"#,
        )
        .unwrap();

        let loaded = Store::with_data_dir(home.join(".roder")).load().unwrap();

        assert_eq!(loaded.access, "");
        assert_eq!(loaded.refresh, "");
        assert_eq!(loaded.account_id, "");
    }

    #[test]
    fn store_restores_roder_owned_tokens_from_backup_when_primary_is_missing() {
        let home = unique_test_home("roder-store-backup-restore");
        let store = Store::with_data_dir(home.join(".roder"));
        store
            .save(Tokens {
                token_type: "bearer".to_string(),
                access: "access-from-backup".to_string(),
                refresh: "refresh-from-backup".to_string(),
                account_id: "acct_backup".to_string(),
                expires: 456,
            })
            .unwrap();

        let primary = home.join(".roder").join("auth").join("codex.json");
        let backup = home.join(".roder").join("auth").join("codex.json.bak");
        assert!(primary.exists());
        assert!(backup.exists());
        std::fs::remove_file(&primary).unwrap();

        let loaded = store.load().unwrap();

        assert_eq!(loaded.access, "access-from-backup");
        assert_eq!(loaded.refresh, "refresh-from-backup");
        assert_eq!(loaded.account_id, "acct_backup");
        assert_eq!(loaded.expires, 456);
        assert!(primary.exists());
    }

    #[test]
    fn store_delete_removes_primary_and_backup_tokens() {
        let home = unique_test_home("roder-store-delete-backup");
        let store = Store::with_data_dir(home.join(".roder"));
        store
            .save(Tokens {
                token_type: "bearer".to_string(),
                access: "access".to_string(),
                refresh: "refresh".to_string(),
                account_id: "acct".to_string(),
                expires: 789,
            })
            .unwrap();

        store.delete().unwrap();

        assert!(!home.join(".roder").join("auth").join("codex.json").exists());
        assert!(
            !home
                .join(".roder")
                .join("auth")
                .join("codex.json.bak")
                .exists()
        );
    }

    fn unique_test_home(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "roder-codex-auth-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
