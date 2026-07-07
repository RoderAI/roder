use std::process::Command;

struct BrowserCommand {
    label: String,
    command: Command,
}

pub(crate) fn open_browser(url: &str) -> anyhow::Result<()> {
    let mut errors = Vec::new();
    for mut candidate in browser_commands(url) {
        match candidate.command.status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => errors.push(format!("{} exited with {status}", candidate.label)),
            Err(err) => errors.push(format!("{} failed: {err}", candidate.label)),
        }
    }
    anyhow::bail!("failed to open browser: {}", errors.join("; "))
}

fn browser_commands(url: &str) -> Vec<BrowserCommand> {
    browser_programs()
        .into_iter()
        .map(|program| program.command(url))
        .collect()
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum BrowserProgram {
    Open,
    XdgOpen,
    WslView,
    Explorer,
    Rundll32,
}

impl BrowserProgram {
    fn command(self, url: &str) -> BrowserCommand {
        match self {
            Self::Open => {
                let mut command = Command::new("open");
                command.arg(url);
                BrowserCommand {
                    label: "open".to_string(),
                    command,
                }
            }
            Self::XdgOpen => {
                let mut command = Command::new("xdg-open");
                command.arg(url);
                BrowserCommand {
                    label: "xdg-open".to_string(),
                    command,
                }
            }
            Self::WslView => {
                let mut command = Command::new("wslview");
                command.arg(url);
                BrowserCommand {
                    label: "wslview".to_string(),
                    command,
                }
            }
            Self::Explorer => {
                let mut command = Command::new("explorer.exe");
                command.arg(url);
                BrowserCommand {
                    label: "explorer.exe".to_string(),
                    command,
                }
            }
            Self::Rundll32 => {
                let mut command = Command::new("rundll32");
                command.arg("url.dll,FileProtocolHandler").arg(url);
                BrowserCommand {
                    label: "rundll32".to_string(),
                    command,
                }
            }
        }
    }
}

fn browser_programs() -> Vec<BrowserProgram> {
    #[cfg(target_os = "macos")]
    {
        vec![BrowserProgram::Open]
    }
    #[cfg(target_os = "windows")]
    {
        vec![BrowserProgram::Rundll32]
    }
    #[cfg(target_os = "linux")]
    {
        if running_under_wsl() {
            vec![
                BrowserProgram::WslView,
                BrowserProgram::Explorer,
                BrowserProgram::XdgOpen,
            ]
        } else {
            vec![BrowserProgram::XdgOpen]
        }
    }
}

#[cfg(target_os = "linux")]
fn running_under_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    fn command_parts(command: &Command) -> (String, Vec<String>) {
        (
            command.get_program().to_string_lossy().into_owned(),
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
        )
    }

    #[test]
    #[cfg(windows)]
    fn windows_browser_command_does_not_shell_split_oauth_url() {
        let url = "https://auth.kimi.com/device?client_id=app";
        let commands = browser_commands(url);
        let (program, args) = command_parts(&commands[0].command);

        assert_eq!(program, "rundll32");
        assert_eq!(args, vec!["url.dll,FileProtocolHandler", url]);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_wsl_browser_commands_prefer_windows_bridge() {
        let url = "https://auth.kimi.com/device?client_id=app";
        let commands = [
            BrowserProgram::WslView.command(url),
            BrowserProgram::Explorer.command(url),
            BrowserProgram::XdgOpen.command(url),
        ];
        let parts = commands
            .iter()
            .map(|command| command_parts(&command.command))
            .collect::<Vec<_>>();

        assert_eq!(parts[0], ("wslview".to_string(), vec![url.to_string()]));
        assert_eq!(
            parts[1],
            ("explorer.exe".to_string(), vec![url.to_string()])
        );
        assert_eq!(parts[2], ("xdg-open".to_string(), vec![url.to_string()]));
    }
}
