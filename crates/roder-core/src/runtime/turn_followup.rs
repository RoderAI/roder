use super::*;

impl Runtime {
    /// Remove a finished turn from the active map and return user steers it
    /// never consumed. Input can arrive after the turn's last inference
    /// boundary, or after `TurnCompleted` but before teardown (a host draining
    /// its queue then sees the old turn as active and steers it). Draining
    /// under the map's write lock pairs with `enqueue_turn_steer`, which pushes
    /// under the read lock, so no steer can land in a removed handle.
    pub(super) async fn take_unconsumed_turn_steers(&self, turn_id: &TurnId) -> Vec<UserMessage> {
        let mut active_turns = self.active_turns.write().await;
        let Some(handle) = active_turns.remove(turn_id) else {
            return Vec::new();
        };
        let steers = std::mem::take(&mut *handle.steers.lock().await);
        drop(active_turns);
        // Mailbox steers are redelivered when the turn's reservations are released.
        steers
            .into_iter()
            .filter(|steer| steer.mailbox_ack.is_none())
            .map(|steer| steer.message)
            .filter(|message| !message.text.trim().is_empty() || !message.images.is_empty())
            .collect()
    }

    /// Deliver user input a finished turn never consumed: steer it into a turn
    /// that has since started on the thread, or start a follow-up turn with the
    /// finished turn's settings. Returns whether the input was handed on.
    pub(super) async fn replay_unconsumed_turn_steers(
        self: &Arc<Self>,
        mut options: StartTurnRequest,
        steers: Vec<UserMessage>,
    ) -> anyhow::Result<bool> {
        if steers.is_empty() {
            return Ok(false);
        }
        let message = steers
            .iter()
            .map(|steer| steer.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let images = steers
            .into_iter()
            .flat_map(|steer| steer.images)
            .collect::<Vec<_>>();
        let thread_id = options.thread_id.clone();
        // A newer turn may already be running; join it rather than racing it.
        if let Some(turn_id) = self.active_turn_for_thread(&thread_id).await
            && self
                .steer_turn(thread_id, turn_id, message.clone(), images.clone())
                .await
                .is_ok()
        {
            return Ok(true);
        }
        options.message = message;
        options.images = images;
        self.start_turn(options).await?;
        Ok(true)
    }
}

#[cfg(test)]
#[path = "turn_followup_tests.rs"]
mod tests;
