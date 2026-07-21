use thiserror::Error;

/// Base error type for SDK-specific failures
#[derive(Debug, Error)]
pub enum ClaudeSDKError {
    #[error("CLI connection error: {0}")]
    CLIConnection(#[from] CLIConnectionError),

    #[error("CLI not found: {0}")]
    CLINotFound(CLINotFoundError),

    #[error("Process error: {0}")]
    Process(#[from] ProcessError),

    #[error("JSON decode error: {0}")]
    CLIJSONDecode(#[from] CLIJSONDecodeError),

    #[error("Message parse error: {0}")]
    MessageParse(#[from] MessageParseError),

    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Session error: {0}")]
    Session(String),

    #[error("MCP error: {0}")]
    MCP(String),

    #[error("Control request error: {subtype} - {message}")]
    ControlRequest { subtype: String, message: String },

    #[error("{0}")]
    Other(String),
}

impl From<CLINotFoundError> for ClaudeSDKError {
    fn from(err: CLINotFoundError) -> Self {
        ClaudeSDKError::CLINotFound(err)
    }
}

/// Reports failures when the SDK cannot connect to the CLI
#[derive(Debug, Error)]
#[error("CLI connection failed: {message}")]
pub struct CLIConnectionError {
    pub message: String,
}

impl CLIConnectionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Reports that the Claude CLI could not be located
#[derive(Debug, Error)]
#[error("CLI not found: {message} (path: {cli_path})")]
pub struct CLINotFoundError {
    pub message: String,
    pub cli_path: String,
}

impl CLINotFoundError {
    pub fn new(message: impl Into<String>, cli_path: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            cli_path: cli_path.into(),
        }
    }
}

/// Reports CLI process failures
#[derive(Debug)]
pub struct ProcessError {
    pub message: String,
    pub exit_code: Option<i32>,
    pub stderr: String,
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(code) = self.exit_code {
            write!(f, " (exit code: {})", code)?;
        }
        if !self.stderr.is_empty() {
            write!(f, "\nError output: {}", self.stderr)?;
        }
        Ok(())
    }
}

impl std::error::Error for ProcessError {}

impl ProcessError {
    pub fn new(
        message: impl Into<String>,
        exit_code: Option<i32>,
        stderr: impl Into<String>,
    ) -> Self {
        Self {
            message: message.into(),
            exit_code,
            stderr: stderr.into(),
        }
    }
}

/// Reports malformed JSON from CLI stdout
#[derive(Debug, Error)]
#[error("Failed to decode JSON: {line}")]
pub struct CLIJSONDecodeError {
    pub line: String,
    #[source]
    pub original_error: serde_json::Error,
}

impl CLIJSONDecodeError {
    pub fn new(line: impl Into<String>, original_error: serde_json::Error) -> Self {
        let line = line.into();
        let truncated = if line.len() > 100 {
            format!("{}...", &line[..100])
        } else {
            line
        };
        Self {
            line: truncated,
            original_error,
        }
    }
}

/// Reports malformed, known CLI payloads
#[derive(Debug, Error)]
#[error("Message parse error: {message}")]
pub struct MessageParseError {
    pub message: String,
    pub data: Option<serde_json::Map<String, serde_json::Value>>,
}

impl MessageParseError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: serde_json::Map<String, serde_json::Value>) -> Self {
        self.data = Some(data);
        self
    }
}

/// Map internal transport errors to public SDK errors
pub fn map_transport_error<E>(err: E) -> ClaudeSDKError
where
    E: std::error::Error + 'static,
{
    // Try downcasting to specific error types
    if let Some(e) =
        (&err as &(dyn std::error::Error + 'static)).downcast_ref::<CLIConnectionError>()
    {
        return ClaudeSDKError::CLIConnection(CLIConnectionError::new(&e.message));
    }

    ClaudeSDKError::Other(err.to_string())
}

/// Result type alias for SDK operations
pub type Result<T> = std::result::Result<T, ClaudeSDKError>;
