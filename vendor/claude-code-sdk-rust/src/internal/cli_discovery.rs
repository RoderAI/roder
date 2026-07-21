use std::path::{Path, PathBuf};

use crate::error::{CLINotFoundError, ClaudeSDKError, Result};

const DEFAULT_CLI_NAME: &str = "claude";
const MINIMUM_CLAUDE_CODE_VERSION: (u64, u64, u64) = (2, 0, 0);

pub(crate) fn find_cli_path(explicit: Option<&str>) -> Result<String> {
    if let Some(path) = explicit {
        return Ok(path.to_string());
    }

    if let Some(path) = bundled_cli_path().filter(|path| path.is_file()) {
        return Ok(path.to_string_lossy().to_string());
    }

    if let Ok(path) = which::which(DEFAULT_CLI_NAME) {
        return Ok(path.to_string_lossy().to_string());
    }

    if let Some(home) = dirs::home_dir() {
        for path in fallback_cli_locations(&home) {
            if path.is_file() {
                return Ok(path.to_string_lossy().to_string());
            }
        }
    }

    Err(ClaudeSDKError::CLINotFound(CLINotFoundError::new(
        "Claude Code not found. Install with npm install -g @anthropic-ai/claude-code, ensure it is on PATH, or set cli_path.",
        DEFAULT_CLI_NAME,
    )))
}

pub(crate) fn fallback_cli_locations(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".npm-global/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
        home.join(".local/bin/claude"),
        home.join("node_modules/.bin/claude"),
        home.join(".yarn/bin/claude"),
        home.join(".claude/local/claude"),
    ]
}

pub(crate) fn parse_cli_version(output: &str) -> Option<(u64, u64, u64)> {
    let version = output.split_whitespace().next()?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

pub(crate) fn is_supported_cli_version(version: (u64, u64, u64)) -> bool {
    version >= MINIMUM_CLAUDE_CODE_VERSION
}

pub(crate) fn unsupported_cli_version_warning(cli_path: &str, version: (u64, u64, u64)) -> String {
    format!(
        "Claude Code version {}.{}.{} at {} is unsupported in the Agent SDK. Minimum required version is {}.{}.{}.",
        version.0,
        version.1,
        version.2,
        cli_path,
        MINIMUM_CLAUDE_CODE_VERSION.0,
        MINIMUM_CLAUDE_CODE_VERSION.1,
        MINIMUM_CLAUDE_CODE_VERSION.2,
    )
}

pub(crate) async fn check_cli_version(cli_path: &str) -> Option<bool> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::process::Command::new(cli_path).arg("-v").output(),
    )
    .await
    .ok()?
    .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = parse_cli_version(stdout.trim())?;
    let supported = is_supported_cli_version(version);
    if !supported {
        tracing::warn!("{}", unsupported_cli_version_warning(cli_path, version));
    }
    Some(supported)
}

fn bundled_cli_path() -> Option<PathBuf> {
    let cli_name = if cfg!(windows) {
        "claude.exe"
    } else {
        DEFAULT_CLI_NAME
    };
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    Some(dir.join("_bundled").join(cli_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_locations_match_python_sdk_order() {
        let home = PathBuf::from("/home/alice");
        let locations = fallback_cli_locations(&home);

        assert_eq!(
            locations[0],
            PathBuf::from("/home/alice/.npm-global/bin/claude")
        );
        assert_eq!(locations[1], PathBuf::from("/usr/local/bin/claude"));
        assert_eq!(locations[2], PathBuf::from("/home/alice/.local/bin/claude"));
        assert_eq!(
            locations[3],
            PathBuf::from("/home/alice/node_modules/.bin/claude")
        );
        assert_eq!(locations[4], PathBuf::from("/home/alice/.yarn/bin/claude"));
        assert_eq!(
            locations[5],
            PathBuf::from("/home/alice/.claude/local/claude")
        );
    }

    #[test]
    fn parses_semver_prefix_from_cli_version_output() {
        assert_eq!(parse_cli_version("2.1.110"), Some((2, 1, 110)));
        assert_eq!(parse_cli_version("2.0.0 (Claude Code)"), Some((2, 0, 0)));
        assert_eq!(parse_cli_version("not-a-version"), None);
    }

    #[test]
    fn checks_minimum_supported_version() {
        assert!(!is_supported_cli_version((1, 9, 99)));
        assert!(is_supported_cli_version((2, 0, 0)));
        assert!(is_supported_cli_version((2, 1, 0)));
    }

    #[test]
    fn unsupported_version_warning_includes_version_path_and_minimum() {
        let warning = unsupported_cli_version_warning("/usr/bin/claude", (1, 9, 99));

        assert!(warning.contains("1.9.99"));
        assert!(warning.contains("/usr/bin/claude"));
        assert!(warning.contains("2.0.0"));
    }
}
