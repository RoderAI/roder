//! Authentication stays with the official CLI; Roder never reads its tokens.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use roder_api::backend::AgentBackend;
use roder_ext_antigravity_backend::{AntigravityBackend, clear_api_key_environment};
use tokio::process::Command;

fn login_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .args(["acp", "--login"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    clear_api_key_environment(&mut command);
    command
}

pub(crate) async fn login() -> anyhow::Result<()> {
    let program = roder_ext_antigravity_backend::runtime::find_runtime()?.context(
        "Antigravity is not installed. Start `roder --backend antigravity` in a terminal for guided installation and sign-in.",
    )?;
    eprintln!("Signing in through the official Antigravity CLI...");
    let status = login_command(&program)
        .status()
        .await
        .context("could not start Antigravity sign-in; start `roder --backend antigravity` for guided setup, then retry `roder auth login antigravity`")?;
    if !status.success() {
        anyhow::bail!(
            "Antigravity sign-in exited with {status}; run `roder auth login antigravity` to retry"
        );
    }
    eprintln!("Antigravity sign-in completed. Start Roder with `roder --backend antigravity`.");
    Ok(())
}

pub(crate) async fn status() -> anyhow::Result<()> {
    // Listing models creates an ACP session, but sends no prompt or inference.
    // Do not inspect credential files or infer authentication from their presence.
    tokio::time::timeout(
        Duration::from_secs(45),
        AntigravityBackend::default().list_models(std::env::current_dir()?),
    )
    .await
    .context("Antigravity authentication probe timed out; run `roder auth login antigravity`")?
    .context("could not verify Antigravity authentication through agy; run `roder auth login antigravity`")?;
    println!("antigravity: authenticated ACP session available");
    Ok(())
}

pub(crate) async fn logout() -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(45),
        AntigravityBackend::default().logout(std::env::current_dir()?),
    )
    .await
    .context("Antigravity logout timed out; credentials may still be active")?
    .context("could not sign out through the official Antigravity ACP logout method")?;
    println!("antigravity: signed out");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_uses_official_command_and_removes_api_credentials() {
        let command = login_command(Path::new("/test/agy"));
        let command = command.as_std();
        assert_eq!(command.get_program(), "/test/agy");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec!["acp", "--login"]
        );
        for name in [
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "GOOGLE_CLOUD_PROJECT",
            "GOOGLE_CLOUD_LOCATION",
            "GOOGLE_CLOUD_QUOTA_PROJECT",
            "GOOGLE_GENAI_USE_VERTEXAI",
            "GOOGLE_GENAI_USE_GCA",
            "GCLOUD_PROJECT",
            "CLOUDSDK_CORE_PROJECT",
            "AGY_ACP_CCPA_PROJECT",
            "AGY_ACP_ENABLE_OAUTH",
        ] {
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| key == name && value.is_none()),
                "{name} must not override subscription authentication"
            );
        }
        assert!(command.get_envs().any(|(key, value)| {
            key == "AGY_ACP_FORCE_FILE_STORAGE" && value == Some(std::ffi::OsStr::new("1"))
        }));
        assert!(command.get_envs().any(|(key, value)| {
            key == "SSH_CONNECTION" && value.is_some_and(|value| !value.is_empty())
        }));
        assert!(
            !command.get_envs().any(
                |(key, value)| key == "BROWSER" && value == Some(std::ffi::OsStr::new("false"))
            ),
            "interactive login must not inherit the backend's browser suppression"
        );
    }
}
