//! Typed Rift adapter errors: `code`, `path`, and a safe message. Raw
//! stderr is bounded and never carries unrelated absolute paths beyond the
//! fork paths the caller already knows.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiftError {
    /// Coarse machine-readable class: `binary_missing`, `command_failed`,
    /// `parse_failed`, `not_found`, `confirmation_mismatch`.
    pub code: &'static str,
    /// The fork/source path the failure relates to, when known.
    pub path: Option<String>,
    pub message: String,
}

impl RiftError {
    pub fn new(code: &'static str, path: Option<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RiftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.path {
            Some(path) => write!(f, "rift {} ({path}): {}", self.code, self.message),
            None => write!(f, "rift {}: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for RiftError {}
