use std::sync::Arc;

use anyhow::{Context, bail};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::subagents::SubagentDefinition;
use roder_api::tools::ToolRegistry;
use roder_ext_subagents::{
    InProcessDispatcher, InProcessDispatcherConfig, InferenceEngineRegistry, SubagentsExtension,
    TaskToolConfig,
};

#[derive(Debug, Clone)]
pub struct DefaultSubagentsConfig {
    pub enabled: bool,
    pub definitions: Vec<SubagentDefinition>,
    pub default_agent: String,
    pub default_provider: Option<String>,
    pub default_model: String,
    pub max_concurrent: usize,
    pub max_depth: usize,
    pub default_timeout_seconds: u64,
    pub include_child_transcript: bool,
    pub expose_per_type: bool,
}

impl Default for DefaultSubagentsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            definitions: Vec::new(),
            default_agent: "explore".to_string(),
            default_provider: None,
            default_model: "mock".to_string(),
            max_concurrent: 2,
            max_depth: 1,
            default_timeout_seconds: 180,
            include_child_transcript: false,
            expose_per_type: false,
        }
    }
}

pub(crate) fn install_subagents(
    builder: &mut ExtensionRegistryBuilder,
    config: DefaultSubagentsConfig,
) -> anyhow::Result<()> {
    if !config.enabled {
        return Ok(());
    }
    let parent_tools = contributed_parent_tools(builder)?;
    validate_subagents_config(builder, &config, &parent_tools)?;

    let engines = inference_engine_registry(builder, &config);
    let dispatcher = Arc::new(InProcessDispatcher::new(
        InProcessDispatcherConfig {
            id: "in-process-subagents".to_string(),
            default_agent: config.default_agent,
            default_provider: config.default_provider,
            default_model: config.default_model,
            max_concurrent: config.max_concurrent,
            max_depth: config.max_depth,
            default_timeout_seconds: config.default_timeout_seconds,
            include_child_transcript: config.include_child_transcript,
            default_max_turns: InProcessDispatcherConfig::default().default_max_turns,
            default_max_result_chars: InProcessDispatcherConfig::default().default_max_result_chars,
        },
        config.definitions,
        engines,
        parent_tools,
    )?);

    builder.install(SubagentsExtension::with_task_tool_config(
        dispatcher,
        TaskToolConfig {
            expose_per_type: config.expose_per_type,
            ..TaskToolConfig::default()
        },
    ))
}

fn validate_subagents_config(
    builder: &ExtensionRegistryBuilder,
    config: &DefaultSubagentsConfig,
    parent_tools: &ToolRegistry,
) -> anyhow::Result<()> {
    if config.definitions.is_empty() {
        bail!("subagents are enabled but no agent definitions were loaded");
    }
    if config.default_agent.trim().is_empty() {
        bail!("subagents.default_agent must not be empty");
    }
    if !config
        .definitions
        .iter()
        .any(|definition| definition.agent_type == config.default_agent)
    {
        bail!(
            "subagents.default_agent {:?} does not match a loaded agent definition",
            config.default_agent
        );
    }
    if config.max_depth == 0 {
        bail!("subagents.max_depth must be at least 1");
    }
    if config.max_concurrent == 0 {
        bail!("subagents.max_concurrent must be at least 1");
    }
    if let Some(provider) = &config.default_provider
        && !builder
            .inference_engines
            .iter()
            .any(|engine| engine.id() == *provider)
    {
        bail!("subagents.default_provider {provider:?} is not registered");
    }
    for definition in &config.definitions {
        for tool in &definition.tools {
            if parent_tools.get(tool).is_none() {
                bail!(
                    "subagent {:?} references unknown tool {:?}",
                    definition.agent_type,
                    tool
                );
            }
        }
    }
    Ok(())
}

fn inference_engine_registry(
    builder: &ExtensionRegistryBuilder,
    config: &DefaultSubagentsConfig,
) -> InferenceEngineRegistry {
    let mut registry = InferenceEngineRegistry::new();
    if let Some(default_provider) = &config.default_provider
        && let Some(engine) = builder
            .inference_engines
            .iter()
            .find(|engine| engine.id() == *default_provider)
    {
        registry.insert(engine.clone());
    }
    for engine in &builder.inference_engines {
        if config.default_provider.as_deref() != Some(engine.id().as_str()) {
            registry.insert(engine.clone());
        }
    }
    registry
}

fn contributed_parent_tools(builder: &ExtensionRegistryBuilder) -> anyhow::Result<ToolRegistry> {
    let mut registry = ToolRegistry::default();
    for contributor in &builder.tools {
        contributor
            .contribute(&mut registry)
            .with_context(|| format!("failed to install tool contributor {}", contributor.id()))?;
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use roder_api::extension::ExtensionRegistry;
    use roder_api::subagents::SubagentPermissionMode;

    use super::*;
    use crate::{DefaultRegistryConfig, build_default_registry};

    #[test]
    fn disabled_subagents_config_does_not_install_task_tool() {
        let registry = build_default_registry(DefaultRegistryConfig {
            subagents: Some(DefaultSubagentsConfig {
                enabled: false,
                definitions: vec![definition("explore", &["echo"])],
                ..DefaultSubagentsConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        })
        .unwrap();
        let names = contributed_tool_names(&registry).unwrap();

        assert!(!names.contains(&"task".to_string()));
    }

    #[test]
    fn enabled_subagents_without_definitions_fails_fast() {
        let err = match build_default_registry(DefaultRegistryConfig {
            subagents: Some(DefaultSubagentsConfig {
                enabled: true,
                ..DefaultSubagentsConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        }) {
            Ok(_) => panic!("enabled subagents without definitions should fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("no agent definitions"));
    }

    #[test]
    fn unknown_default_agent_fails_fast() {
        let err = match build_default_registry(DefaultRegistryConfig {
            subagents: Some(DefaultSubagentsConfig {
                enabled: true,
                definitions: vec![definition("review", &["echo"])],
                default_agent: "explore".to_string(),
                ..DefaultSubagentsConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        }) {
            Ok(_) => panic!("unknown default agent should fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("default_agent"));
        assert!(err.to_string().contains("explore"));
    }

    #[test]
    fn zero_depth_fails_fast() {
        let err = match build_default_registry(DefaultRegistryConfig {
            subagents: Some(DefaultSubagentsConfig {
                enabled: true,
                definitions: vec![definition("explore", &["echo"])],
                max_depth: 0,
                ..DefaultSubagentsConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        }) {
            Ok(_) => panic!("zero max_depth should fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("max_depth"));
        assert!(err.to_string().contains("at least 1"));
    }

    #[test]
    fn unknown_agent_tool_fails_fast() {
        let err = match build_default_registry(DefaultRegistryConfig {
            subagents: Some(DefaultSubagentsConfig {
                enabled: true,
                definitions: vec![definition("explore", &["missing_tool"])],
                ..DefaultSubagentsConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        }) {
            Ok(_) => panic!("unknown subagent tool should fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("unknown tool"));
        assert!(err.to_string().contains("missing_tool"));
    }

    #[test]
    fn enabled_subagents_install_task_tool() {
        let registry = build_default_registry(DefaultRegistryConfig {
            subagents: Some(DefaultSubagentsConfig {
                enabled: true,
                definitions: vec![definition("explore", &["echo"])],
                ..DefaultSubagentsConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        })
        .unwrap();
        let names = contributed_tool_names(&registry).unwrap();

        assert!(names.contains(&"task".to_string()));
        assert!(!names.contains(&"task_explore".to_string()));
    }

    #[test]
    fn enabled_subagents_can_install_per_type_task_tools() {
        let registry = build_default_registry(DefaultRegistryConfig {
            subagents: Some(DefaultSubagentsConfig {
                enabled: true,
                definitions: vec![definition("explore", &["echo"])],
                expose_per_type: true,
                ..DefaultSubagentsConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        })
        .unwrap();
        let names = contributed_tool_names(&registry).unwrap();

        assert!(names.contains(&"task".to_string()));
        assert!(names.contains(&"task_explore".to_string()));
    }

    fn definition(agent_type: &str, tools: &[&str]) -> SubagentDefinition {
        SubagentDefinition {
            agent_type: agent_type.to_string(),
            description: format!("{agent_type} agent"),
            tools: tools.iter().map(|tool| tool.to_string()).collect(),
            model: None,
            system_prompt: Some("Return a concise result.".to_string()),
            permission_mode: SubagentPermissionMode::Default,
            max_turns: Some(1),
            max_result_chars: Some(1000),
        }
    }

    fn contributed_tool_names(registry: &ExtensionRegistry) -> anyhow::Result<Vec<String>> {
        let mut tools = ToolRegistry::default();
        for contributor in &registry.tools {
            contributor.contribute(&mut tools)?;
        }
        Ok(tools.specs().into_iter().map(|spec| spec.name).collect())
    }
}
