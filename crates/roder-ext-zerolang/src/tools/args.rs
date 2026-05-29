use serde::Deserialize;

use crate::patch::GraphPatchOperation;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillsGetArgs {
    #[serde(default)]
    pub skill: Option<String>,
    #[serde(default)]
    pub full: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InputArgs {
    pub input: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub emit: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GraphOutputArgs {
    pub input: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub out: Option<String>,
    #[serde(default)]
    pub allow_outside_zero: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FixPlanArgs {
    pub input: String,
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EditArgs {
    pub input: String,
    pub graph_hash: String,
    pub operations: Vec<GraphPatchOperation>,
    #[serde(default)]
    pub out: Option<String>,
    #[serde(default)]
    pub allow_outside_zero: bool,
    #[serde(default = "default_true")]
    pub validate: bool,
}

fn default_true() -> bool {
    true
}
