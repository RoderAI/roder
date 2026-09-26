use agent_client_protocol_schema as acp;
use roder_api::patch_progress::PatchProgress;

pub(super) fn acp_update(progress: PatchProgress) -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
        progress.tool_id,
        acp::ToolCallUpdateFields::new().raw_input(serde_json::json!({"patch":progress.patch})),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::RoderEvent;
    use roder_api::patch_progress::ProposedPatchChange;

    #[test]
    fn proposed_patch_progress_has_stable_native_and_acp_wire_shapes() {
        let progress = PatchProgress {
            thread_id: "thread1".into(),
            turn_id: "turn1".into(),
            tool_id: "patch1".into(),
            patch: "*** Begin Patch\n*** Add File: café.txt\n+hello\n*** End Patch".into(),
            changes: vec![ProposedPatchChange {
                path: "café.txt".into(),
                change_type: "add".into(),
                move_to: None,
                old_lines: vec![],
                new_lines: vec!["hello".into()],
            }],
            complete: true,
        };
        let event = RoderEvent::PatchProgress(progress.clone());
        assert_eq!(event.kind(), "patch/progress");
        assert_eq!(event.thread_id(), Some(&progress.thread_id));
        assert_eq!(event.turn_id(), Some(&progress.turn_id));
        let notifications = crate::notifications::protocol_notifications_for_event(&event);
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].method, "item/applyPatch/progress");
        assert_eq!(notifications[0].params["toolId"], "patch1");
        assert_eq!(
            notifications[0].params["changes"][0]["newLines"],
            serde_json::json!(["hello"])
        );
        assert!(
            notifications[0].params["changes"][0]
                .get("moveTo")
                .is_none()
        );
        let update = crate::acp::acp_session_update("session1", &notifications[0]).unwrap();
        assert_eq!(update.method, "session/update");
        assert_eq!(update.params["update"]["sessionUpdate"], "tool_call_update");
        assert_eq!(update.params["update"]["toolCallId"], "patch1");
        assert_eq!(update.params["update"]["rawInput"]["patch"], progress.patch);
        assert!(
            update.params["update"].get("status").is_none(),
            "generation completion must not claim a filesystem write"
        );
    }
}
