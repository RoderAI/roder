use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use roder_api::subagents::SubagentDefinition;
use tokio::fs;

use crate::agent_def::parse_agent_definition;

#[derive(Debug, Clone, Default)]
pub struct AgentLoadConfig {
    pub user_dir: Option<PathBuf>,
    pub workspace_dir: Option<PathBuf>,
}

pub async fn load_agent_definitions(
    config: &AgentLoadConfig,
) -> anyhow::Result<Vec<SubagentDefinition>> {
    let mut definitions = BTreeMap::new();
    if let Some(user_dir) = &config.user_dir {
        load_dir(user_dir, &mut definitions).await?;
    }
    if let Some(workspace_dir) = &config.workspace_dir {
        load_dir(workspace_dir, &mut definitions).await?;
    }
    Ok(definitions.into_values().collect())
}

async fn load_dir(
    dir: &Path,
    definitions: &mut BTreeMap<String, SubagentDefinition>,
) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(dir)
        .await
        .with_context(|| format!("failed to read agent directory {}", dir.display()))?;
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            files.push(path);
        }
    }
    files.sort();

    for path in files {
        let markdown = fs::read_to_string(&path)
            .await
            .with_context(|| format!("failed to read agent definition {}", path.display()))?;
        let definition = parse_agent_definition(&markdown)
            .with_context(|| format!("failed to parse agent definition {}", path.display()))?;
        definitions.insert(definition.agent_type.clone(), definition);
    }
    Ok(())
}
