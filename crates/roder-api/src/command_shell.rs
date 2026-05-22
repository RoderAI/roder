use std::path::Path;

pub fn default_command_shell() -> String {
    default_command_shell_for(
        std::env::var("SHELL").ok().as_deref(),
        cfg!(target_os = "macos"),
    )
}

pub fn normalize_command_shell(shell: &str) -> Option<String> {
    let shell = shell.trim();
    (!shell.is_empty()).then(|| shell.to_string())
}

pub fn command_shell_options(active: &str) -> Vec<String> {
    let mut shells = Vec::new();
    push_unique_shell(&mut shells, active);
    push_unique_shell(&mut shells, "zsh");
    push_unique_shell(&mut shells, "bash");
    shells
}

fn push_unique_shell(shells: &mut Vec<String>, shell: &str) {
    if let Some(shell) = normalize_command_shell(shell)
        && !shells.iter().any(|known| known == &shell)
    {
        shells.push(shell);
    }
}

fn default_command_shell_for(login_shell: Option<&str>, is_macos: bool) -> String {
    if let Some(login_shell) = login_shell.and_then(normalize_command_shell)
        && shell_name(&login_shell) == Some("zsh")
    {
        return login_shell;
    }

    if is_macos {
        "/bin/zsh".to_string()
    } else {
        "bash".to_string()
    }
}

fn shell_name(shell: &str) -> Option<&str> {
    Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .or_else(|| (!shell.trim().is_empty()).then_some(shell))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_prefers_zsh_login_shell() {
        assert_eq!(
            default_command_shell_for(Some("/usr/local/bin/zsh"), false),
            "/usr/local/bin/zsh"
        );
    }

    #[test]
    fn default_uses_macos_zsh_when_login_shell_is_not_zsh() {
        assert_eq!(
            default_command_shell_for(Some("/usr/local/bin/fish"), true),
            "/bin/zsh"
        );
    }

    #[test]
    fn default_uses_bash_off_macos_when_login_shell_is_not_zsh() {
        assert_eq!(
            default_command_shell_for(Some("/usr/local/bin/fish"), false),
            "bash"
        );
    }

    #[test]
    fn command_shell_options_include_active_and_common_shells_once() {
        assert_eq!(
            command_shell_options("/opt/homebrew/bin/fish"),
            vec![
                "/opt/homebrew/bin/fish".to_string(),
                "zsh".to_string(),
                "bash".to_string(),
            ]
        );
        assert_eq!(
            command_shell_options("zsh"),
            vec!["zsh".to_string(), "bash".to_string()]
        );
    }
}
