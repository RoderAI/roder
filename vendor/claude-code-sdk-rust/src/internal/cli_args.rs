use crate::error::{ClaudeSDKError, Result};
use crate::types::{SettingSource, SkillsConfig};

use super::transport::TransportOptions;

fn serialize_cli_value<T: serde::Serialize>(value: &T) -> Result<String> {
    let value = serde_json::to_value(value)?;
    match value {
        serde_json::Value::String(s) => Ok(s),
        other => Ok(other.to_string()),
    }
}

fn serialize_setting_sources(sources: &[SettingSource]) -> Result<String> {
    sources
        .iter()
        .map(serialize_cli_value)
        .collect::<Result<Vec<_>>>()
        .map(|sources| sources.join(","))
}

fn effective_allowed_tools(options: &TransportOptions) -> Vec<String> {
    let mut allowed_tools = options.allowed_tools.clone();

    match &options.skills {
        Some(SkillsConfig::All) if !allowed_tools.iter().any(|tool| tool == "Skill") => {
            allowed_tools.push("Skill".to_string());
        }
        Some(SkillsConfig::Names(names)) => {
            for name in names {
                let pattern = format!("Skill({name})");
                if !allowed_tools.iter().any(|tool| tool == &pattern) {
                    allowed_tools.push(pattern);
                }
            }
        }
        Some(SkillsConfig::All) | None => {}
    }

    allowed_tools
}

fn effective_setting_sources(options: &TransportOptions) -> Option<Vec<SettingSource>> {
    if let Some(sources) = &options.setting_sources {
        return Some(sources.clone());
    }

    options
        .skills
        .as_ref()
        .map(|_| vec![SettingSource::User, SettingSource::Project])
}

fn read_settings_file(path: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&content).ok()
}

fn parse_settings_value(settings: &str) -> serde_json::Map<String, serde_json::Value> {
    let trimmed = settings.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(trimmed)
            .unwrap_or_else(|_| read_settings_file(trimmed).unwrap_or_default())
    } else {
        read_settings_file(trimmed).unwrap_or_default()
    }
}

fn build_settings_value(options: &TransportOptions) -> Result<Option<String>> {
    let Some(sandbox) = &options.sandbox else {
        return Ok(options.settings.clone());
    };

    let mut settings = options
        .settings
        .as_deref()
        .map(parse_settings_value)
        .unwrap_or_default();
    settings.insert("sandbox".to_string(), serde_json::to_value(sandbox)?);

    Ok(Some(serde_json::Value::Object(settings).to_string()))
}

pub(crate) fn build_cli_args(options: &TransportOptions) -> Result<Vec<String>> {
    let mut args = vec![
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
    ];

    if let Some(ref file) = options.system_prompt_file {
        args.push("--system-prompt-file".to_string());
        args.push(file.path.clone());
    } else if let Some(ref preset) = options.system_prompt_preset {
        if let Some(ref append) = preset.append {
            args.push("--append-system-prompt".to_string());
            args.push(append.clone());
        }
    } else {
        let prompt = options.system_prompt.as_deref().unwrap_or("");
        args.push("--system-prompt".to_string());
        args.push(prompt.to_string());
    }

    if let Some(ref preset) = options.tools_preset {
        let preset_name = if preset.preset.is_empty() || preset.preset == "claude_code" {
            "default"
        } else {
            &preset.preset
        };
        args.push("--tools".to_string());
        args.push(preset_name.to_string());
    } else if options.tools_set {
        args.push("--tools".to_string());
        args.push(options.tools.join(","));
    }

    let allowed_tools = effective_allowed_tools(options);
    if !allowed_tools.is_empty() {
        args.push("--allowedTools".to_string());
        args.push(allowed_tools.join(","));
    }

    if !options.disallowed_tools.is_empty() {
        args.push("--disallowedTools".to_string());
        args.push(options.disallowed_tools.join(","));
    }

    if let Some(turns) = options.max_turns {
        args.push("--max-turns".to_string());
        args.push(turns.to_string());
    }

    if let Some(budget) = options.max_budget_usd {
        args.push("--max-budget-usd".to_string());
        args.push(budget.to_string());
    }

    if let Some(ref model) = options.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }

    if let Some(ref fallback) = options.fallback_model {
        args.push("--fallback-model".to_string());
        args.push(fallback.clone());
    }

    if !options.betas.is_empty() {
        args.push("--betas".to_string());
        let betas: Vec<String> = options
            .betas
            .iter()
            .map(serialize_cli_value)
            .collect::<Result<Vec<_>>>()?;
        args.push(betas.join(","));
    }

    if let Some(ref name) = options.permission_prompt_tool_name {
        args.push("--permission-prompt-tool".to_string());
        args.push(name.clone());
    }

    if let Some(mode) = options.permission_mode {
        args.push("--permission-mode".to_string());
        args.push(serialize_cli_value(&mode)?);
    }

    if options.continue_conversation {
        args.push("--continue".to_string());
    }

    if let Some(ref resume) = options.resume {
        args.push("--resume".to_string());
        args.push(resume.clone());
    }

    if let Some(ref session_id) = options.session_id {
        args.push("--session-id".to_string());
        args.push(session_id.clone());
    }

    if let Some(ref task_budget) = options.task_budget {
        args.push("--task-budget".to_string());
        args.push(task_budget.total.to_string());
    }

    if let Some(settings) = build_settings_value(options)? {
        args.push("--settings".to_string());
        args.push(settings);
    }

    for dir in &options.add_dirs {
        args.push("--add-dir".to_string());
        args.push(dir.clone());
    }

    if let Some(config) = &options.mcp_servers_config {
        args.push("--mcp-config".to_string());
        args.push(config.clone());
    } else if !options.mcp_servers.is_empty() {
        let mcp_config = serde_json::json!({
            "mcpServers": options.mcp_servers
        });
        args.push("--mcp-config".to_string());
        args.push(mcp_config.to_string());
    }

    if options.include_partial_messages {
        args.push("--include-partial-messages".to_string());
    }

    if options.include_hook_events {
        args.push("--include-hook-events".to_string());
    }

    if options.strict_mcp_config {
        args.push("--strict-mcp-config".to_string());
    }

    if options.fork_session {
        args.push("--fork-session".to_string());
    }

    if options.session_store_enabled {
        args.push("--session-mirror".to_string());
    }

    if let Some(setting_sources) = effective_setting_sources(options) {
        let setting_sources = serialize_setting_sources(&setting_sources)?;
        args.push(format!("--setting-sources={setting_sources}"));
    }

    for plugin in &options.plugins {
        if plugin.r#type != "local" {
            return Err(ClaudeSDKError::Other(format!(
                "Unsupported plugin type: {}",
                plugin.r#type
            )));
        }
        if !plugin.path.is_empty() {
            args.push("--plugin-dir".to_string());
            args.push(plugin.path.clone());
        }
    }

    if let Some(ref thinking) = options.thinking {
        match thinking.r#type {
            crate::types::ThinkingConfigType::Adaptive => {
                args.push("--thinking".to_string());
                args.push("adaptive".to_string());
            }
            crate::types::ThinkingConfigType::Enabled => {
                if let Some(tokens) = thinking.budget_tokens {
                    args.push("--max-thinking-tokens".to_string());
                    args.push(tokens.to_string());
                }
            }
            crate::types::ThinkingConfigType::Disabled => {
                args.push("--thinking".to_string());
                args.push("disabled".to_string());
            }
        }
        if thinking.r#type != crate::types::ThinkingConfigType::Disabled {
            if let Some(ref display) = thinking.display {
                args.push("--thinking-display".to_string());
                args.push(display.clone());
            }
        }
    } else if let Some(tokens) = options.max_thinking_tokens {
        args.push("--max-thinking-tokens".to_string());
        args.push(tokens.to_string());
    }

    if let Some(effort) = options.effort {
        args.push("--effort".to_string());
        args.push(effort.as_cli().to_string());
    }

    if let Some(ref output_format) = options.output_format {
        if output_format.get("type").and_then(|v| v.as_str()) == Some("json_schema") {
            if let Some(schema) = output_format.get("schema") {
                args.push("--json-schema".to_string());
                args.push(schema.to_string());
            }
        }
    }

    let mut extra_keys: Vec<&String> = options.extra_args.keys().collect();
    extra_keys.sort();
    for key in extra_keys {
        args.push(format!("--{}", key));
        if let Some(ref value) = options.extra_args[key] {
            args.push(value.clone());
        }
    }

    Ok(args)
}
