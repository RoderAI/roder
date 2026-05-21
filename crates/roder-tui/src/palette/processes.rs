use roder_api::processes::{ProcessDescriptor, ProcessState};

use super::{PaletteAction, PaletteItem, StaticPaletteSource};

pub fn process_source(processes: &[ProcessDescriptor]) -> StaticPaletteSource {
    let mut entries = vec![
        (
            PaletteItem {
                id: "processes-open".to_string(),
                title: "Processes".to_string(),
                subtitle: Some("List Roder-owned shell and background processes".to_string()),
                keywords: vec![
                    "ps".to_string(),
                    "processes".to_string(),
                    "tasks".to_string(),
                ],
                icon: Some('P'),
            },
            PaletteAction::ShowProcesses,
        ),
        (
            PaletteItem {
                id: "processes-stop-all".to_string(),
                title: "Stop all processes".to_string(),
                subtitle: Some("Prepare the explicit /ps stop-all confirmation".to_string()),
                keywords: vec!["ps".to_string(), "stop".to_string(), "all".to_string()],
                icon: Some('!'),
            },
            PaletteAction::InsertComposerText("/ps stop-all --confirm".to_string()),
        ),
    ];

    for process in processes {
        let state = state_label(&process.state);
        let short = short(&process.process_id);
        entries.push((
            PaletteItem {
                id: format!("process-detail-{}", process.process_id),
                title: format!("Process {short}"),
                subtitle: Some(format!("{state} · {}", process.command_summary)),
                keywords: vec![
                    "ps".to_string(),
                    "process".to_string(),
                    process.process_id.clone(),
                    process.command_summary.clone(),
                ],
                icon: Some('P'),
            },
            PaletteAction::ShowProcessDetail(process.process_id.clone()),
        ));
        if process.stoppable
            && matches!(
                process.state,
                ProcessState::Running | ProcessState::Starting
            )
        {
            entries.push((
                PaletteItem {
                    id: format!("process-stop-{}", process.process_id),
                    title: format!("Stop process {short}"),
                    subtitle: Some(process.command_summary.clone()),
                    keywords: vec![
                        "ps".to_string(),
                        "stop".to_string(),
                        process.process_id.clone(),
                    ],
                    icon: Some('x'),
                },
                PaletteAction::StopProcess(process.process_id.clone()),
            ));
        }
    }

    StaticPaletteSource::new("processes", "Processes", entries)
}

fn state_label(state: &ProcessState) -> &'static str {
    match state {
        ProcessState::Starting => "starting",
        ProcessState::Running => "running",
        ProcessState::Stopping => "stopping",
        ProcessState::Exited { .. } => "exited",
        ProcessState::Failed { .. } => "failed",
        ProcessState::Stopped => "stopped",
    }
}

fn short(id: &str) -> &str {
    let end = id.len().min(8);
    &id[..end]
}

#[cfg(test)]
mod tests {
    use roder_api::processes::{ProcessOrigin, ProcessState};
    use time::OffsetDateTime;

    use super::*;

    #[test]
    fn palette_processes_source_exposes_open_detail_and_stop_rows() {
        let source = process_source(&[ProcessDescriptor {
            process_id: "process-123".to_string(),
            origin: ProcessOrigin::CommandExec,
            state: ProcessState::Running,
            command: vec!["sleep".to_string(), "10".to_string()],
            command_summary: "sleep 10".to_string(),
            cwd: None,
            pid: None,
            task_id: None,
            thread_id: None,
            turn_id: None,
            runner_destination_id: None,
            runner_session_id: None,
            stoppable: true,
            started_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            stdout_tail: None,
            stderr_tail: None,
        }]);
        let entries = source.entries();

        assert!(entries.iter().any(|entry| entry.item.title == "Processes"));
        assert!(
            entries
                .iter()
                .any(|entry| matches!(entry.action, PaletteAction::ShowProcessDetail(_)))
        );
        assert!(
            entries
                .iter()
                .any(|entry| matches!(entry.action, PaletteAction::StopProcess(_)))
        );
    }
}
