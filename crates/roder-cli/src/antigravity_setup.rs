//! Interactive setup happens before the main TUI initializes its backend.

use std::io::IsTerminal;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use roder_api::backend::AgentBackend;
use roder_ext_antigravity_backend::AntigravityBackend;
use roder_ext_antigravity_backend::runtime::{self, InstallProgress};
use roder_tui::provider_setup::{ProviderSetupDriver, SetupCheck, SetupStatus, run_provider_setup};
use tokio::sync::mpsc::UnboundedSender;

const CHECK_TIMEOUT: Duration = Duration::from_secs(45);

struct AntigravitySetup;

#[async_trait]
impl ProviderSetupDriver for AntigravitySetup {
    async fn check(&self) -> anyhow::Result<SetupCheck> {
        let program = match runtime::find_runtime() {
            Ok(Some(program)) => program,
            Ok(None) => {
                return Ok(SetupCheck {
                    status: SetupStatus::MissingRuntime,
                    detail: format!(
                        "The official Antigravity CLI was not found. Roder can download and verify it in {} without sudo or changing your PATH.",
                        runtime::managed_directory()?.display()
                    ),
                });
            }
            Err(error) => {
                return Ok(SetupCheck {
                    status: SetupStatus::MissingRuntime,
                    detail: format!(
                        "The Antigravity installation could not be located or is damaged: {error:#}. Install a verified managed runtime to repair it.",
                    ),
                });
            }
        };
        let version = match runtime::runtime_version(&program).await {
            Ok(version) => version,
            Err(error) => {
                return Ok(SetupCheck {
                    status: SetupStatus::MissingRuntime,
                    detail: format!(
                        "The Antigravity runtime at {} could not be verified: {error:#}. Install a managed copy to continue.",
                        program.display()
                    ),
                });
            }
        };
        let backend = AntigravityBackend::with_program(program);
        match tokio::time::timeout(CHECK_TIMEOUT, backend.list_models(std::env::current_dir()?))
            .await
        {
            Ok(Ok(models)) => Ok(SetupCheck {
                status: SetupStatus::Ready,
                detail: format!(
                    "Antigravity {version} is ready. Your Google account offers {} models; choose one in Roder's model picker.",
                    models.len()
                ),
            }),
            Ok(Err(error)) => Ok(SetupCheck {
                status: SetupStatus::NeedsSignIn,
                detail: format!(
                    "Antigravity {version} is installed. Google account access could not be verified: {error} Sign in with your subscription account, or press r to retry if this is a temporary connection problem."
                ),
            }),
            Err(_) => Ok(SetupCheck {
                status: SetupStatus::NeedsSignIn,
                detail: format!(
                    "Antigravity {version} is installed, but verifying Google access timed out. Sign in or retry after checking your connection."
                ),
            }),
        }
    }

    async fn install(&self, progress: UnboundedSender<String>) -> anyhow::Result<()> {
        runtime::install_runtime(move |event| {
            let _ = progress.send(progress_text(event));
        })
        .await?;
        Ok(())
    }

    async fn sign_in(&self) -> anyhow::Result<()> {
        crate::antigravity_auth::login().await
    }
}

fn progress_text(progress: InstallProgress) -> String {
    match progress {
        InstallProgress::Manifest => "Checking Google's official release manifest...".into(),
        InstallProgress::Downloading { bytes, total } => {
            let downloaded = bytes / (1024 * 1024);
            if let Some(total) = total {
                format!(
                    "Downloading Antigravity: {downloaded} / {} MiB",
                    total / (1024 * 1024)
                )
            } else {
                format!("Downloading Antigravity: {downloaded} MiB")
            }
        }
        InstallProgress::Extracting => "Verifying and extracting the official runtime...".into(),
        InstallProgress::Verifying => "Checking the installed Antigravity version...".into(),
        InstallProgress::Installed { path, version } => {
            format!("Antigravity {version} installed at {}", path.display())
        }
    }
}

pub(crate) async fn prepare_tui() -> anyhow::Result<bool> {
    let driver = Arc::new(AntigravitySetup);
    let initial = driver.check().await?;
    if matches!(initial.status, SetupStatus::Ready) {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        anyhow::bail!(
            "Antigravity setup requires an interactive terminal. {} Start `roder --backend antigravity` in a terminal for guided installation and Google sign-in. No download or sign-in was started.",
            initial.detail
        );
    }
    run_provider_setup(driver, initial).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_progress_works_with_or_without_content_length() {
        assert_eq!(
            progress_text(InstallProgress::Downloading {
                bytes: 3 * 1024 * 1024,
                total: Some(10 * 1024 * 1024),
            }),
            "Downloading Antigravity: 3 / 10 MiB"
        );
        assert_eq!(
            progress_text(InstallProgress::Downloading {
                bytes: 3 * 1024 * 1024,
                total: None,
            }),
            "Downloading Antigravity: 3 MiB"
        );
    }
}
