use roder_protocol::RunnerStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerUiState {
    LocalFallback,
    Connecting,
    Active,
    PausedResumable,
    Failed,
}

pub fn runner_ui_state(status: Option<&RunnerStatus>) -> RunnerUiState {
    match status.map(|status| status.state.as_str()) {
        None => RunnerUiState::LocalFallback,
        Some("connecting") | Some("configured") => RunnerUiState::Connecting,
        Some("active") => RunnerUiState::Active,
        Some("paused") | Some("resumable") => RunnerUiState::PausedResumable,
        Some("failed") => RunnerUiState::Failed,
        Some(_) => RunnerUiState::Failed,
    }
}

pub fn runner_status_label(status: Option<&RunnerStatus>) -> String {
    match runner_ui_state(status) {
        RunnerUiState::LocalFallback => "runner:local".to_string(),
        RunnerUiState::Connecting => status
            .map(|status| format!("runner:{}:connecting", status.destination_id))
            .unwrap_or_else(|| "runner:local".to_string()),
        RunnerUiState::Active => status
            .map(|status| format!("runner:{}:active", status.destination_id))
            .unwrap_or_else(|| "runner:local".to_string()),
        RunnerUiState::PausedResumable => status
            .map(|status| format!("runner:{}:resumable", status.destination_id))
            .unwrap_or_else(|| "runner:local".to_string()),
        RunnerUiState::Failed => status
            .map(|status| format!("runner:{}:failed", status.destination_id))
            .unwrap_or_else(|| "runner:failed".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_states_cover_required_tui_states() {
        assert_eq!(runner_ui_state(None), RunnerUiState::LocalFallback);
        assert_eq!(
            runner_ui_state(Some(&status("unix-local", "configured"))),
            RunnerUiState::Connecting
        );
        assert_eq!(
            runner_ui_state(Some(&status("unix-local", "active"))),
            RunnerUiState::Active
        );
        assert_eq!(
            runner_ui_state(Some(&status("unix-local", "paused"))),
            RunnerUiState::PausedResumable
        );
        assert_eq!(
            runner_ui_state(Some(&status("unix-local", "failed"))),
            RunnerUiState::Failed
        );
    }

    fn status(destination_id: &str, state: &str) -> RunnerStatus {
        RunnerStatus {
            destination_id: destination_id.to_string(),
            provider_id: destination_id.to_string(),
            state: state.to_string(),
            session_id: None,
        }
    }
}
