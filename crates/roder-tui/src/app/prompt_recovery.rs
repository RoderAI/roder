use super::*;

impl<C: AppClient> TuiApp<C> {
    pub(super) async fn run_retry_slash_command(&mut self, args: &str) {
        let thread_id = self.focused_thread_id().to_owned();
        match args.trim() {
            "discard" => {
                if self.prompt_admission.take_recovery(&thread_id).is_some() {
                    self.timeline.push_system(
                        "Discarded one retained input; queued follow-ups were preserved.",
                    );
                    self.push_event("discarded retained input".into());
                } else {
                    self.record_error("no retained input to discard".into());
                }
                return;
            }
            "" => {}
            _ => {
                self.record_error("usage: /retry [discard]".into());
                return;
            }
        }
        if self.active_turn_id.is_some() {
            self.record_error("retry unavailable while a turn is running; interrupt first.".into());
            return;
        }
        let recovered = self.prompt_admission.take_recovery(&thread_id);
        let pending = recovered.or_else(|| {
            self.last_user_prompt
                .clone()
                .filter(|prompt| prompt.thread_id.as_deref().is_none_or(|id| id == thread_id))
                .map(|mut prompt| {
                    prompt.renew_client_id();
                    prompt
                })
        });
        let Some(pending) = pending else {
            self.record_error("nothing to retry yet.".into());
            return;
        };
        self.push_event("slash command: /retry".into());
        self.retry_prepared_prompt(pending).await;
    }
}
