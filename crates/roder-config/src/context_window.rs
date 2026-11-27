use std::path::Path;

/// Persist the GPT/Codex context override without changing unrelated settings.
pub fn save_model_context_window(window: Option<u32>) -> anyhow::Result<()> {
    save_model_context_window_to_path(crate::config_path(), window)
}

pub fn save_model_context_window_to_path(
    path: impl AsRef<Path>,
    window: Option<u32>,
) -> anyhow::Result<()> {
    anyhow::ensure!(window != Some(0), "model_context_window must be positive");
    let path = path.as_ref();
    let mut config = crate::load_config_file_from_path(path)?;
    config.model_context_window = window;
    crate::save_config_file_to_path(path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_override_persists_and_resets_without_dropping_settings() {
        let dir = std::env::temp_dir().join(format!("roder-context-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "model = 'gpt-6.2-sol'\n").unwrap();
        save_model_context_window_to_path(&path, Some(500_000)).unwrap();
        let cfg = crate::load_config_file_from_path(&path).unwrap();
        assert_eq!(cfg.model_context_window, Some(500_000));
        assert_eq!(cfg.model.as_deref(), Some("gpt-6.2-sol"));
        save_model_context_window_to_path(&path, None).unwrap();
        assert_eq!(
            crate::load_config_file_from_path(&path)
                .unwrap()
                .model_context_window,
            None
        );
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("model_context_window")
        );
        assert!(save_model_context_window_to_path(&path, Some(0)).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
