use std::io::Write as _;
use std::time::Duration;

pub const DEFAULT_AGENT_SERVICE_URL: &str = "https://agentn.global.api5.cursor.sh";
pub const DEFAULT_AGENT_SERVICE_PATH: &str = "/agent.v1.AgentService/Run";
pub const DEFAULT_CLIENT_VERSION: &str = "cli-2026.05.24-dda726e";

#[derive(Debug, Clone)]
pub struct AgentServiceConfig {
    pub endpoint: String,
    pub path: String,
    pub client_version: String,
    pub timeout: Duration,
}

impl Default for AgentServiceConfig {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_AGENT_SERVICE_URL.to_string(),
            path: DEFAULT_AGENT_SERVICE_PATH.to_string(),
            client_version: DEFAULT_CLIENT_VERSION.to_string(),
            timeout: Duration::from_secs(60),
        }
    }
}

/// Env var pointing at a JSONL file. When set, every raw Cursor AgentService
/// wire frame (outbound request frames and inbound response payloads) is
/// appended as hex. This is a diagnostic used to reverse-engineer Cursor-native
/// tool-call frames (e.g. `edit`/`shell`) so they can be mapped into canonical
/// Roder tool execution. It has zero effect on normal runs (no var = no-op).
pub const CAPTURE_FRAMES_ENV: &str = "RODER_CURSOR_CAPTURE_FRAMES";

pub(crate) fn capture_cursor_frame(direction: &str, index: usize, bytes: &[u8]) {
    let path = match std::env::var(CAPTURE_FRAMES_ENV) {
        Ok(path) if !path.is_empty() => path,
        _ => return,
    };
    capture_frame_at(std::path::Path::new(&path), direction, index, bytes);
}

fn capture_frame_at(path: &std::path::Path, direction: &str, index: usize, bytes: &[u8]) {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = std::fmt::Write::write_fmt(&mut hex, format_args!("{byte:02x}"));
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!(
        "{{\"ts_ms\":{ts},\"dir\":\"{direction}\",\"index\":{index},\"len\":{},\"hex\":\"{hex}\"}}\n",
        bytes.len()
    );
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_agentservice_endpoint() {
        let config = AgentServiceConfig::default();
        assert_eq!(config.endpoint, DEFAULT_AGENT_SERVICE_URL);
        assert_eq!(config.path, DEFAULT_AGENT_SERVICE_PATH);
    }

    #[test]
    fn capture_frame_appends_hex_jsonl_to_requested_path() {
        // With the env var set, frames are appended as hex JSONL lines.
        let mut path = std::env::temp_dir();
        path.push(format!(
            "roder-cursor-capture-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        capture_frame_at(&path, "send", 3, &[0x00, 0x01, 0xff]);
        capture_frame_at(&path, "recv", 0, &[0xab]);

        let contents = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"dir\":\"send\""));
        assert!(lines[0].contains("\"index\":3"));
        assert!(lines[0].contains("\"len\":3"));
        assert!(lines[0].contains("\"hex\":\"0001ff\""));
        assert!(lines[1].contains("\"hex\":\"ab\""));
    }
}
