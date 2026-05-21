use crate::fixture::{FileBackedGradingRules, LoadedFileBackedFixture};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileBackedContextMetrics {
    pub inline_chars: usize,
    pub full_output_chars: usize,
    pub inline_chars_saved: usize,
    pub artifact_reads: u32,
    pub artifact_grep_calls: u32,
    pub answer_correct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileBackedGrade {
    pub fixture_id: String,
    pub passed: bool,
    pub answer_correct: bool,
    pub secret_recoverable: bool,
    pub secret_not_in_inline: bool,
    pub used_artifact_follow_up: bool,
    pub metrics: FileBackedContextMetrics,
    pub failures: Vec<String>,
}

/// Normalized answer check (trim, lowercase, collapse whitespace).
pub fn grade_file_backed_answer(expected: &str, actual: &str) -> bool {
    normalize_answer(expected) == normalize_answer(actual)
}

fn normalize_answer(text: &str) -> String {
    text.trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn secret_in_text(rules: &FileBackedGradingRules, text: &str) -> bool {
    if let Some(pattern) = &rules.secret_pattern {
        if text.contains(pattern) {
            return true;
        }
    }
    false
}

pub fn grade_file_backed_fixture(
    loaded: &LoadedFileBackedFixture,
    actual_answer: &str,
    artifact_reads: u32,
    artifact_grep_calls: u32,
) -> anyhow::Result<FileBackedGrade> {
    let inline = loaded.read_inline()?;
    let full_output = loaded.read_full_output()?.unwrap_or_else(|| {
        loaded
            .fixture
            .context
            .artifacts
            .first()
            .map(|_| String::new())
            .unwrap_or_default()
    });
    let full_output_chars = if full_output.is_empty() {
        loaded
            .fixture
            .context
            .artifacts
            .iter()
            .map(|artifact| loaded.read_artifact(&artifact.artifact_id))
            .next()
            .transpose()?
            .map(|text| text.len())
            .unwrap_or(0)
    } else {
        full_output.len()
    };

    grade_file_backed_trajectory(
        loaded,
        &inline,
        full_output_chars,
        actual_answer,
        artifact_reads,
        artifact_grep_calls,
    )
}

pub fn grade_file_backed_trajectory(
    loaded: &LoadedFileBackedFixture,
    inline: &str,
    full_output_chars: usize,
    actual_answer: &str,
    artifact_reads: u32,
    artifact_grep_calls: u32,
) -> anyhow::Result<FileBackedGrade> {
    let fixture = &loaded.fixture;
    let rules = &fixture.grading;
    let mut failures = Vec::new();

    let answer_correct = grade_file_backed_answer(&fixture.expected_answer, actual_answer);
    if fixture.metrics.track_answer_correctness && !answer_correct {
        failures.push(format!(
            "expected answer {:?}, got {:?}",
            fixture.expected_answer, actual_answer
        ));
    }

    let secret_not_in_inline =
        !rules.secret_must_not_appear_in_inline || !secret_in_text(rules, inline);
    if rules.secret_must_not_appear_in_inline && !secret_not_in_inline {
        failures.push("secret must not appear in inline context".to_string());
    }

    let secret_recoverable = recoverable_from_artifacts(loaded, rules)?;
    if rules.require_artifact_follow_up && !secret_recoverable {
        failures.push("expected answer not recoverable from artifacts".to_string());
    }

    let used_artifact_follow_up = artifact_reads > 0 || artifact_grep_calls > 0;
    if rules.require_artifact_follow_up && !used_artifact_follow_up {
        failures.push("expected artifact read or grep follow-up".to_string());
    }

    let inline_chars = inline.len();
    let inline_chars_saved = full_output_chars.saturating_sub(inline_chars);
    let metrics = FileBackedContextMetrics {
        inline_chars,
        full_output_chars,
        inline_chars_saved,
        artifact_reads,
        artifact_grep_calls,
        answer_correct,
    };

    let passed = failures.is_empty();
    Ok(FileBackedGrade {
        fixture_id: fixture.id.clone(),
        passed,
        answer_correct,
        secret_recoverable,
        secret_not_in_inline,
        used_artifact_follow_up,
        metrics,
        failures,
    })
}

fn recoverable_from_artifacts(
    loaded: &LoadedFileBackedFixture,
    rules: &FileBackedGradingRules,
) -> anyhow::Result<bool> {
    let expected = &loaded.fixture.expected_answer;
    for artifact in &loaded.fixture.context.artifacts {
        let body = loaded.read_artifact(&artifact.artifact_id)?;
        if body.contains(expected) {
            return Ok(true);
        }
        if let Some(pattern) = &rules.secret_pattern {
            if body.contains(pattern) {
                return Ok(true);
            }
        }
        if let Some(line) = rules.secret_line {
            let line_text = body
                .lines()
                .nth(line.saturating_sub(1) as usize)
                .unwrap_or_default();
            if line_text.contains(expected) || secret_in_text(rules, line_text) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{default_file_backed_suite_dir, load_file_backed_suite};

    #[test]
    fn file_backed_context_long_command_requires_grep() {
        let dir = default_file_backed_suite_dir();
        let (_, fixtures) = load_file_backed_suite(&dir).unwrap();
        let loaded = fixtures
            .into_iter()
            .find(|f| f.fixture.id == "long-command-secret-line")
            .unwrap();
        let inline = loaded.read_inline().unwrap();
        assert!(!inline.contains("deploy-token-7f3a2c91"));

        let fail = grade_file_backed_fixture(&loaded, "unknown", 0, 0).unwrap();
        assert!(!fail.passed);
        assert!(!fail.answer_correct);

        let pass = grade_file_backed_fixture(&loaded, "deploy-token-7f3a2c91", 0, 1).unwrap();
        assert!(pass.passed);
        assert!(pass.metrics.inline_chars_saved > 10_000);
        assert_eq!(pass.metrics.artifact_grep_calls, 1);
    }

    #[test]
    fn file_backed_context_compaction_history_recovery() {
        let dir = default_file_backed_suite_dir();
        let (_, fixtures) = load_file_backed_suite(&dir).unwrap();
        let loaded = fixtures
            .into_iter()
            .find(|f| f.fixture.id == "compaction-history-recovery")
            .unwrap();
        let inline = loaded.read_inline().unwrap();
        assert!(!inline.contains("a1b2c3d4-e5f6-7890-abcd-ef1234567890"));

        let pass = grade_file_backed_fixture(
            &loaded,
            "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            1,
            0,
        )
        .unwrap();
        assert!(pass.passed);
        assert!(pass.secret_recoverable);
        assert!(pass.secret_not_in_inline);
    }

    #[test]
    fn file_backed_context_answer_normalization() {
        assert!(grade_file_backed_answer(
            "deploy-token-7f3a2c91",
            "  DEPLOY-TOKEN-7f3a2c91 \n"
        ));
    }
}
