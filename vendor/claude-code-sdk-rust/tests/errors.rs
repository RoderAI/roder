use claude_code_sdk_rust::{
    CLIConnectionError, CLIJSONDecodeError, CLINotFoundError, ClaudeSDKError, ProcessError,
};

#[test]
fn cli_not_found_error_formats_message_and_path() {
    let error = CLINotFoundError::new("Claude Code not found", "/missing/claude");
    let sdk_error = ClaudeSDKError::from(error);

    let message = sdk_error.to_string();
    assert!(message.contains("Claude Code not found"));
    assert!(message.contains("/missing/claude"));
}

#[test]
fn connection_error_formats_message() {
    let sdk_error = ClaudeSDKError::from(CLIConnectionError::new("failed to connect"));

    assert!(sdk_error.to_string().contains("failed to connect"));
}

#[test]
fn process_error_preserves_exit_code_and_stderr() {
    let error = ProcessError::new("Process failed", Some(1), "Command not found");

    assert_eq!(error.exit_code, Some(1));
    assert_eq!(error.stderr, "Command not found");
    let message = error.to_string();
    assert!(message.contains("Process failed"));
    assert!(message.contains("exit code: 1"));
    assert!(message.contains("Command not found"));
}

#[test]
fn json_decode_error_preserves_short_line_and_source() {
    let original = serde_json::from_str::<serde_json::Value>("{invalid json}").unwrap_err();
    let error = CLIJSONDecodeError::new("{invalid json}", original);

    assert_eq!(error.line, "{invalid json}");
    assert!(error.to_string().contains("Failed to decode JSON"));
}
