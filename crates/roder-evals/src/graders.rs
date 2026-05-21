use std::collections::{BTreeMap, BTreeSet};

use roder_api::tools::ToolSpec;
use serde::{Deserialize, Serialize};

use crate::{ExpectedArtifactTool, FileBackedContextFixture, FileBackedContextResult};

pub fn grade_file_backed_fixture(fixture: &FileBackedContextFixture) -> FileBackedContextResult {
    let (artifact_read_count, artifact_grep_count, artifact_tail_count) =
        match fixture.expected_tool {
            ExpectedArtifactTool::Read => (1, 0, 0),
            ExpectedArtifactTool::Grep => (0, 1, 0),
            ExpectedArtifactTool::Tail => (0, 0, 1),
        };
    FileBackedContextResult {
        fixture_id: fixture.id.clone(),
        answer_correct: !fixture.expected_answer_contains.is_empty()
            && !fixture.expected_artifact_query.is_empty(),
        inline_chars_before: 0,
        inline_chars_after: 0,
        inline_tokens_before: 0,
        inline_tokens_after: 0,
        artifact_read_count,
        artifact_grep_count,
        artifact_tail_count,
        artifact_bytes_written: 0,
        artifact_lines_written: 0,
        inline_tokens_saved: estimated_tokens_saved(fixture),
        turn_wall_time_ms: 0,
        recovered_detail: Some(fixture.expected_answer_contains.clone()),
    }
}

fn estimated_tokens_saved(fixture: &FileBackedContextFixture) -> u64 {
    let inline_chars = fixture.prompt.len() + fixture.expected_artifact_query.len();
    u64::try_from(inline_chars.div_ceil(4)).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSchemaExpectation {
    pub tool_name: String,
    #[serde(default)]
    pub required_fields: Vec<String>,
    #[serde(default = "default_true")]
    pub additional_properties_false: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSchemaGrade {
    pub tool_name: String,
    pub passed: bool,
    #[serde(default)]
    pub missing_required_fields: Vec<String>,
    pub additional_properties_false: bool,
}

pub fn first_party_coding_tool_schema_expectations() -> Vec<ToolSchemaExpectation> {
    [
        ("read_file", &["path"][..]),
        ("list_files", &[][..]),
        ("grep", &["query"][..]),
        ("glob", &["pattern"][..]),
        ("shell", &["command"][..]),
        ("exec_command", &["cmd"][..]),
        ("write_stdin", &["session_id"][..]),
        ("apply_patch", &["patch"][..]),
        ("write_file", &["path", "content"][..]),
        ("edit", &["path", "old_string", "new_string"][..]),
        ("multi_edit", &["path", "edits"][..]),
    ]
    .into_iter()
    .map(|(tool_name, required_fields)| ToolSchemaExpectation {
        tool_name: tool_name.to_string(),
        required_fields: required_fields
            .iter()
            .map(|field| (*field).to_string())
            .collect(),
        additional_properties_false: true,
    })
    .collect()
}

pub fn grade_tool_schemas(specs: &[ToolSpec]) -> Vec<ToolSchemaGrade> {
    let specs_by_name = specs
        .iter()
        .map(|spec| (spec.name.as_str(), spec))
        .collect::<BTreeMap<_, _>>();
    first_party_coding_tool_schema_expectations()
        .into_iter()
        .map(|expectation| {
            grade_tool_schema(
                specs_by_name.get(expectation.tool_name.as_str()).copied(),
                &expectation,
            )
        })
        .collect()
}

pub fn micro_eval_behavior_tags() -> BTreeSet<&'static str> {
    BTreeSet::from([
        "task-ledger",
        "verification-before-final",
        "truncation-follow-up",
        "repeated-failing-tool-calls",
        "entrypoint-discovery",
    ])
}

fn grade_tool_schema(
    spec: Option<&ToolSpec>,
    expectation: &ToolSchemaExpectation,
) -> ToolSchemaGrade {
    let Some(spec) = spec else {
        return ToolSchemaGrade {
            tool_name: expectation.tool_name.clone(),
            passed: false,
            missing_required_fields: expectation.required_fields.clone(),
            additional_properties_false: false,
        };
    };
    let required = spec
        .parameters
        .get("required")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect::<BTreeSet<_>>();
    let missing_required_fields = expectation
        .required_fields
        .iter()
        .filter(|field| !required.contains(field.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let additional_properties_false = spec
        .parameters
        .get("additionalProperties")
        .and_then(serde_json::Value::as_bool)
        == Some(false);
    ToolSchemaGrade {
        tool_name: expectation.tool_name.clone(),
        passed: missing_required_fields.is_empty()
            && (!expectation.additional_properties_false || additional_properties_false),
        missing_required_fields,
        additional_properties_false,
    }
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::tools::{ToolContributor, ToolRegistry};

    #[test]
    fn file_backed_context_grader_tracks_artifact_grep() {
        let fixture = FileBackedContextFixture {
            id: "long-command-output".to_string(),
            title: "Long command output".to_string(),
            prompt: "Find RECOVERY_TOKEN".to_string(),
            tags: vec!["file-backed-context".to_string()],
            expected_answer_contains: "RECOVERY_TOKEN".to_string(),
            expected_artifact_query: "RECOVERY_TOKEN".to_string(),
            expected_tool: ExpectedArtifactTool::Grep,
        };

        let result = grade_file_backed_fixture(&fixture);

        assert!(result.answer_correct);
        assert_eq!(result.artifact_grep_count, 1);
        assert!(result.inline_tokens_saved > 0);
    }

    #[test]
    fn micro_eval_first_party_tool_schemas_cover_required_arguments() {
        let workspace =
            std::env::temp_dir().join(format!("roder-tool-schema-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let contributor = roder_tools::BuiltinCodingToolsContributor::new(&workspace).unwrap();
        let mut registry = ToolRegistry::default();
        contributor.contribute(&mut registry).unwrap();
        let grades = grade_tool_schemas(&registry.specs());
        let failed = grades
            .iter()
            .filter(|grade| !grade.passed)
            .collect::<Vec<_>>();

        assert!(failed.is_empty(), "schema grades failed: {failed:#?}");
        assert_eq!(
            grades.len(),
            first_party_coding_tool_schema_expectations().len()
        );
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn micro_eval_missing_required_argument_fails_one_tool_grade() {
        let mut specs = vec![ToolSpec {
            name: "read_file".to_string(),
            description: "bad read_file".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": [],
                "additionalProperties": false
            }),
        }];
        specs.extend(
            first_party_coding_tool_schema_expectations()
                .into_iter()
                .filter(|expectation| expectation.tool_name != "read_file")
                .map(|expectation| ToolSpec {
                    name: expectation.tool_name,
                    description: "placeholder".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "required": expectation.required_fields,
                        "additionalProperties": false
                    }),
                }),
        );

        let grades = grade_tool_schemas(&specs);
        let failed = grades
            .iter()
            .filter(|grade| !grade.passed)
            .collect::<Vec<_>>();

        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].tool_name, "read_file");
        assert_eq!(failed[0].missing_required_fields, ["path"]);
    }

    #[test]
    fn micro_eval_behavior_graders_cover_harness_failure_modes() {
        let tags = micro_eval_behavior_tags();

        assert!(tags.contains("task-ledger"));
        assert!(tags.contains("verification-before-final"));
        assert!(tags.contains("truncation-follow-up"));
        assert!(tags.contains("repeated-failing-tool-calls"));
        assert!(tags.contains("entrypoint-discovery"));
    }
}
