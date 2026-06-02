use std::path::PathBuf;

use anyhow::Result;
use roder_api::skills::FeatureSkillBinding;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
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
    pub workflow: Option<WorkflowCommandSpec>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowCommandSpec {
    pub script_id: String,
    pub script_hash: String,
    pub host_api_version: u32,
    pub arguments_schema: Value,
    pub body: Option<String>,
}

impl WorkflowCommandSpec {
    pub fn structured_arguments(&self, arguments: &str) -> Result<Value> {
        structured_workflow_arguments(&self.arguments_schema, arguments)
    }
}

pub fn structured_workflow_arguments(schema: &Value, arguments: &str) -> Result<Value> {
    let arguments = arguments.trim();
    if arguments.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    if let Ok(value) = serde_json::from_str::<Value>(arguments) {
        return Ok(value);
    }

    let mut object = Map::new();
    let key = preferred_schema_text_field(schema).unwrap_or("arguments");
    object.insert(key.to_string(), Value::String(arguments.to_string()));
    Ok(Value::Object(object))
}

fn preferred_schema_text_field(schema: &Value) -> Option<&str> {
    let properties = schema.get("properties")?.as_object()?;
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for field in required {
            let Some(field) = field.as_str() else {
                continue;
            };
            if properties.contains_key(field) {
                return Some(field);
            }
        }
    }
    if properties.len() == 1 {
        return properties.keys().next().map(String::as_str);
    }
    ["question", "query", "prompt", "task"]
        .into_iter()
        .find(|field| properties.contains_key(*field))
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
