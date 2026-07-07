use std::process::Command as StdCommand;

use tokio::process::Command;

struct BrowserCommand {
    label: String,
    command: Command,
}

struct DetachedBrowserCommand {
    label: String,
    command: StdCommand,
}

pub(crate) async fn open_url(url: &str) -> anyhow::Result<()> {
    let mut errors = Vec::new();
    for mut candidate in browser_commands(url) {
        match candidate.command.spawn() {
            Ok(mut child) => match child.wait().await {
                Ok(status) if status.success() => return Ok(()),
                Ok(status) => errors.push(format!("{} exited with {status}", candidate.label)),
                Err(err) => errors.push(format!("{} wait failed: {err}", candidate.label)),
            },
            Err(err) => errors.push(format!("{} failed: {err}", candidate.label)),
        }
    }
    anyhow::bail!("failed to open browser: {}", errors.join("; "))
}

pub(crate) fn open_url_detached(url: &str) -> anyhow::Result<()> {
    let mut errors = Vec::new();
    for mut candidate in detached_browser_commands(url) {
        match candidate.command.spawn() {
            Ok(_) => return Ok(()),
            Err(err) => errors.push(format!("{} failed: {err}", candidate.label)),
        }
    }
    anyhow::bail!("failed to open browser: {}", errors.join("; "))
}

fn browser_commands(url: &str) -> Vec<BrowserCommand> {
    browser_programs()
        .into_iter()
        .map(|program| BrowserCommand {
            label: program.label().to_string(),
            command: program.tokio_command(url),
        })
        .collect()
}

fn detached_browser_commands(url: &str) -> Vec<DetachedBrowserCommand> {
    browser_programs()
        .into_iter()
        .map(|program| DetachedBrowserCommand {
            label: program.label().to_string(),
            command: program.std_command(url),
        })
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
    fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::XdgOpen => "xdg-open",
            Self::WslView => "wslview",
            Self::Explorer => "explorer.exe",
            Self::Rundll32 => "rundll32",
        }
    }

    fn tokio_command(self, url: &str) -> Command {
        let mut command = Command::new(self.label());
        self.add_args(&mut command, url);
        command
    }

    fn std_command(self, url: &str) -> StdCommand {
        let mut command = StdCommand::new(self.label());
        self.add_args(&mut command, url);
        command
    }

    fn add_args<C: CommandArgs>(self, command: &mut C, url: &str) {
        if matches!(self, Self::Rundll32) {
            command.arg("url.dll,FileProtocolHandler");
        }
        command.arg(url);
    }
}

trait CommandArgs {
    fn arg(&mut self, arg: &str) -> &mut Self;
}

impl CommandArgs for Command {
    fn arg(&mut self, arg: &str) -> &mut Self {
        Command::arg(self, arg)
    }
}

impl CommandArgs for StdCommand {
    fn arg(&mut self, arg: &str) -> &mut Self {
        StdCommand::arg(self, arg)
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
    fn command_parts(command: &StdCommand) -> (String, Vec<String>) {
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
    fn windows_browser_command_does_not_shell_split_url() {
        let url = "https://example.com/auth?client_id=app";
        let command = BrowserProgram::Rundll32.std_command(url);
        let (program, args) = command_parts(&command);

        assert_eq!(program, "rundll32");
        assert_eq!(args, vec!["url.dll,FileProtocolHandler", url]);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_wsl_browser_commands_prefer_windows_bridge() {
        let url = "https://example.com/auth?client_id=app";
        let commands = [
            BrowserProgram::WslView.std_command(url),
            BrowserProgram::Explorer.std_command(url),
            BrowserProgram::XdgOpen.std_command(url),
        ];
        let parts = commands.iter().map(command_parts).collect::<Vec<_>>();

        assert_eq!(parts[0], ("wslview".to_string(), vec![url.to_string()]));
        assert_eq!(
            parts[1],
            ("explorer.exe".to_string(), vec![url.to_string()])
        );
        assert_eq!(parts[2], ("xdg-open".to_string(), vec![url.to_string()]));
    }
}
