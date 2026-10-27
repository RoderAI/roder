use super::*;

impl<C: AppClient> TuiApp<C> {
    /// Pixel protocols do not participate in the normal text-cell z-order.
    /// Hide owned placements before any overlay can cover the transcript.
    pub(super) fn image_previews_visible(&self) -> bool {
        self.roadmap_mode.is_none()
            && !self.show_palette
            && !self.show_provider_popup
            && !self.show_shortcuts_dialog
            && !self.agent_activity.is_open()
            && !self.workflows.overlay_visible()
            && self.plugin_browser.is_none()
            && self.chrome_panel.is_none()
            && self.review_panel.is_none()
            && self.confirm_dialog.is_none()
            && self.user_input_dialog.is_none()
            && self.tool_detail_modal.is_none()
            && !self.context_counter_hovered
            && !self.source_copy_active()
    }
}
