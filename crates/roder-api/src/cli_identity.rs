//! The command-line program a Roder runtime is embedded in.
//!
//! Roder's own CLI is `roder`, but other programs embed the runtime and ship
//! their own command line: a hint such as "run `roder auth login codex`" means
//! nothing to their users. An embedding program sets its identity once at
//! startup with [`set_cli_identity`]; every user-facing sign-in hint and the
//! sign-in token folder then follow it. Without one, Roder's defaults apply.

use std::path::PathBuf;
use std::sync::OnceLock;

/// The `{provider}` placeholder in [`CliIdentity::auth_login`].
pub const PROVIDER_PLACEHOLDER: &str = "{provider}";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliIdentity {
    /// The program name users type, such as `roder`.
    pub name: String,
    /// The command that signs in to a provider, with `{provider}` where the
    /// provider id goes, such as `roder auth login {provider}`.
    pub auth_login: String,
    /// Where provider sign-in tokens are kept. `None` keeps Roder's data
    /// folder (`RODER_DATA_DIR`, else `~/.roder`).
    pub auth_dir: Option<PathBuf>,
}

impl CliIdentity {
    /// The command a user runs to sign in to `provider` (`codex`, `supergrok`, ...).
    pub fn auth_login_command(&self, provider: &str) -> String {
        self.auth_login.replace(PROVIDER_PLACEHOLDER, provider)
    }
}

impl Default for CliIdentity {
    fn default() -> Self {
        Self {
            name: "roder".to_string(),
            auth_login: format!("roder auth login {PROVIDER_PLACEHOLDER}"),
            auth_dir: None,
        }
    }
}

static IDENTITY: OnceLock<CliIdentity> = OnceLock::new();

/// Name the program embedding Roder, before any provider or sign-in runs.
/// The first identity set wins; a later one is returned as the error.
pub fn set_cli_identity(identity: CliIdentity) -> Result<(), CliIdentity> {
    IDENTITY.set(identity)
}

/// The embedding program's identity, or Roder's own.
pub fn cli_identity() -> &'static CliIdentity {
    IDENTITY.get_or_init(CliIdentity::default)
}

/// The command a user of this program runs to sign in to `provider`.
pub fn auth_login_command(provider: &str) -> String {
    cli_identity().auth_login_command(provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sign_in_command_names_the_provider() {
        assert_eq!(
            CliIdentity::default().auth_login_command("codex"),
            "roder auth login codex"
        );
        let embedded = CliIdentity {
            name: "com".to_string(),
            auth_login: "com agent login {provider}".to_string(),
            auth_dir: None,
        };
        assert_eq!(
            embedded.auth_login_command("codex"),
            "com agent login codex"
        );
    }
}
