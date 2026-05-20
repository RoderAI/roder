#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RoadmapModeState {
    pub selected_plan: Option<String>,
}

impl RoadmapModeState {
    pub fn new(selected_plan: Option<String>) -> Self {
        Self { selected_plan }
    }

    pub fn label(&self) -> String {
        self.selected_plan
            .as_deref()
            .and_then(|path| path.rsplit('/').next())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("select")
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roadmap_mode_label_uses_file_name() {
        let state = RoadmapModeState::new(Some("roadmap/20-roadmapping-mode.md".to_string()));

        assert_eq!(state.label(), "20-roadmapping-mode.md");
    }

    #[test]
    fn roadmap_mode_label_prompts_selection_without_plan() {
        let state = RoadmapModeState::new(None);

        assert_eq!(state.label(), "select");
    }
}
