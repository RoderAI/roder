use super::*;
use ratatui_image::{FontSize, picker::ProtocolType};

fn terminal() -> TerminalImages {
    TerminalImages {
        protocol: ProtocolType::Kitty,
        cell: FontSize::new(8, 16),
        tmux: false,
    }
}

#[test]
fn stale_resize_completions_do_not_replace_current_layout() {
    let (jobs, job_receiver) = mpsc::sync_channel(WORK_QUEUE);
    let (sender, results) = mpsc::sync_channel(WORK_QUEUE);
    let mut previews = ImagePreviews {
        terminal: Some(terminal()),
        worker: Some(Worker { jobs, results }),
        ..ImagePreviews::default()
    };
    previews.attach(0, [PathBuf::from("image.png")]);
    let entry = &mut previews.entries.get_mut(&0).unwrap()[0];
    entry.width = 80;
    entry.pending = true;
    assert!(
        sender
            .send(Completion {
                id: entry.id,
                width: 80,
                image: Err("obsolete result".into()),
            })
            .is_ok()
    );
    previews.update(40);
    let entry = &previews.entries[&0][0];
    assert_eq!(entry.width, 40);
    assert!(entry.pending);
    assert!(entry.error.is_none());
    assert_eq!(job_receiver.try_recv().unwrap().width, 40);
}

#[test]
fn a_stopped_worker_becomes_a_metadata_fallback_not_permanent_loading() {
    let (jobs, _job_receiver) = mpsc::sync_channel(WORK_QUEUE);
    let (sender, results) = mpsc::sync_channel(WORK_QUEUE);
    let mut previews = ImagePreviews {
        terminal: Some(terminal()),
        worker: Some(Worker { jobs, results }),
        ..ImagePreviews::default()
    };
    previews.attach(0, [PathBuf::from("image.png")]);
    previews.entries.get_mut(&0).unwrap()[0].pending = true;
    drop(sender);
    assert!(previews.update(40));
    assert!(previews.terminal.is_none());
    assert!(!previews.entries[&0][0].pending);
    assert!(
        previews.entries[&0][0]
            .error
            .as_deref()
            .unwrap()
            .contains("worker stopped")
    );
}

#[test]
fn ordinary_tools_do_not_create_empty_image_state_or_start_workers() {
    let mut previews = ImagePreviews {
        terminal: Some(terminal()),
        ..ImagePreviews::default()
    };
    previews.attach(0, Vec::<PathBuf>::new());
    assert!(!previews.update(80));
    assert!(previews.entries.is_empty());
    assert!(previews.worker.is_none());
}

#[test]
fn idle_worker_completion_requests_exactly_one_redraw() {
    let (jobs, _job_receiver) = mpsc::sync_channel(WORK_QUEUE);
    let (sender, results) = mpsc::sync_channel(WORK_QUEUE);
    let mut previews = ImagePreviews {
        terminal: Some(terminal()),
        worker: Some(Worker { jobs, results }),
        ..ImagePreviews::default()
    };
    previews.attach(0, [PathBuf::from("image.png")]);
    let entry = &mut previews.entries.get_mut(&0).unwrap()[0];
    entry.width = 40;
    entry.pending = true;
    assert!(
        sender
            .send(Completion {
                id: entry.id,
                width: 40,
                image: Err("decoding failed".into()),
            })
            .is_ok()
    );
    assert!(
        previews.update(40),
        "completion must wake an otherwise idle UI"
    );
    assert!(
        !previews.update(40),
        "an idle unchanged UI must not repaint forever"
    );
}
