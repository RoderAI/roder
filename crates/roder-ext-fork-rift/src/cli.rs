//! Narrow Rift CLI adapter.
//!
//! Adapter command contract (kept deliberately small because upstream Rift
//! is pre-1.0; revisit when Rift stabilizes):
//!
//! ```text
//! rift init                          # idempotent, cwd = source workspace
//! rift create <name> --dest <dir>    # prints the created snapshot path
//! rift list                          # one "<name>\t<path>" line per fork
//! rift remove <path>                 # removes the snapshot at <path>
//! rift gc                            # provider garbage collection
//! ```

use std::path::Path;

use tokio::process::Command;

use crate::config::RiftConfig;
use crate::errors::RiftError;

const MAX_STDERR: usize = 400;

pub async fn run_rift(config: &RiftConfig, cwd: &Path, args: &[&str]) -> Result<String, RiftError> {
    let output = Command::new(&config.rift_bin)
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|err| {
            RiftError::new(
                "binary_missing",
                Some(config.rift_bin.display().to_string()),
                format!("failed to run rift ({err}); set RODER_RIFT_BIN or install rift"),
            )
        })?;
    if !output.status.success() {
        let mut stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.len() > MAX_STDERR {
            stderr.truncate(MAX_STDERR);
            stderr.push('…');
        }
        return Err(RiftError::new(
            "command_failed",
            Some(cwd.display().to_string()),
            format!(
                "rift {} exited with {}: {stderr}",
                args.join(" "),
                output.status
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Parses `rift list` output (`<name>\t<path>` per line).
pub fn parse_list(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let (name, path) = line.split_once('\t')?;
            let name = name.trim();
            let path = path.trim();
            (!name.is_empty() && !path.is_empty()).then(|| (name.to_string(), path.to_string()))
        })
        .collect()
}

/// Extracts the created snapshot path: the last non-empty stdout line.
pub fn parse_created_path(output: &str) -> Result<String, RiftError> {
    output
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            RiftError::new(
                "parse_failed",
                None,
                "rift create produced no output; expected the created snapshot path",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_and_create_parsing_are_strict() {
        let listed = parse_list("a\t/tmp/forks/a\n\nmalformed-line\nb\t/tmp/forks/b\n");
        assert_eq!(
            listed,
            vec![
                ("a".to_string(), "/tmp/forks/a".to_string()),
                ("b".to_string(), "/tmp/forks/b".to_string()),
            ]
        );

        assert_eq!(
            parse_created_path("creating snapshot...\n/tmp/forks/a\n").unwrap(),
            "/tmp/forks/a"
        );
        assert_eq!(parse_created_path("\n\n").unwrap_err().code, "parse_failed");
    }
}
