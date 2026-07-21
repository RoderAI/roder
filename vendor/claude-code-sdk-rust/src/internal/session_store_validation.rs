use crate::error::{ClaudeSDKError, Result};
use crate::types::ClaudeAgentOptions;

pub fn validate_session_store_options(options: &ClaudeAgentOptions) -> Result<()> {
    if options.can_use_tool.is_some() && options.permission_prompt_tool_name.is_some() {
        return Err(ClaudeSDKError::Other(
            "can_use_tool callback cannot be used with permission_prompt_tool_name. \
             Please use one or the other."
                .to_string(),
        ));
    }

    let Some(store) = &options.session_store else {
        return Ok(());
    };

    if options.continue_conversation && options.resume.is_none() && !store.supports_list_sessions()
    {
        return Err(ClaudeSDKError::Other(
            "continue_conversation with session_store requires the store to implement list_sessions()"
                .to_string(),
        ));
    }

    if options.enable_file_checkpointing {
        return Err(ClaudeSDKError::Other(
            "session_store cannot be combined with enable_file_checkpointing \
             (checkpoints are local-disk only and would diverge from the mirrored transcript)"
                .to_string(),
        ));
    }

    Ok(())
}
