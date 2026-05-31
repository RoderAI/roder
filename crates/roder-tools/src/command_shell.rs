use roder_api::tools::ToolExecutionContext;

pub(crate) fn shell_for_context(ctx: &ToolExecutionContext, fallback: &str) -> String {
    ctx.command_shell
        .as_deref()
        .and_then(roder_api::command_shell::normalize_command_shell)
        .unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn command_args_for_shell(shell: &str, command: &str, login: bool) -> Vec<String> {
    if is_powershell(shell) {
        let mut args = vec!["-NoProfile".to_string()];
        if cfg!(windows) {
            args.extend(["-ExecutionPolicy".to_string(), "Bypass".to_string()]);
        }
        args.extend(["-Command".to_string(), command.to_string()]);
        return args;
    }

    vec![
        if login { "-lc" } else { "-c" }.to_string(),
        command.to_string(),
    ]
}

fn is_powershell(shell: &str) -> bool {
    let name = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .trim()
        .trim_end_matches(".exe")
        .to_ascii_lowercase();
    matches!(name.as_str(), "powershell" | "pwsh")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_uses_command_argument_instead_of_unix_shell_flags() {
        let args = command_args_for_shell("powershell", "pnpm test", true);

        assert!(args.contains(&"-Command".to_string()));
        assert!(args.contains(&"pnpm test".to_string()));
        assert!(!args.contains(&"-lc".to_string()));
        assert!(!args.contains(&"-c".to_string()));
    }

    #[test]
    fn pwsh_path_is_detected_as_powershell() {
        let args = command_args_for_shell(r"C:\Program Files\PowerShell\7\pwsh.exe", "dir", false);

        assert!(args.contains(&"-Command".to_string()));
        assert!(!args.contains(&"-c".to_string()));
    }

    #[test]
    fn unix_shells_keep_login_and_non_login_flags() {
        assert_eq!(
            command_args_for_shell("bash", "printf ok", true),
            vec!["-lc".to_string(), "printf ok".to_string()]
        );
        assert_eq!(
            command_args_for_shell("/bin/sh", "printf ok", false),
            vec!["-c".to_string(), "printf ok".to_string()]
        );
    }
}
