use std::path::PathBuf;

use roder_api::skills::FeatureSkillBinding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub name: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub include: CommandInclude,
    pub feature_skill_bindings: Vec<FeatureSkillBinding>,
    pub body: String,
    pub source: CommandSource,
    pub path: Option<PathBuf>,
}

impl CommandSpec {
    pub fn display_source(&self) -> String {
        match &self.source {
            CommandSource::BuiltIn => "built-in".to_string(),
            CommandSource::User => "user".to_string(),
            CommandSource::Workspace => "workspace".to_string(),
            CommandSource::Extension { extension_id } => format!("extension:{extension_id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandSource {
    BuiltIn,
    User,
    Workspace,
    Extension { extension_id: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandInclude {
    pub files: Vec<FileInclude>,
    pub shell: Vec<ShellInclude>,
    pub urls: Vec<UrlInclude>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInclude {
    pub id: Option<String>,
    pub path: String,
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellInclude {
    pub id: Option<String>,
    pub command: String,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlInclude {
    pub id: Option<String>,
    pub url: String,
    pub optional: bool,
}
