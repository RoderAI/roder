//! Turns a [`ReviewTarget`] into the concrete prompt the review turn receives.
//!
//! Everything here goes through the [`VcsProvider`] trait: the git repository
//! type is crate-private to `roder-ext-git`, so merge-base and head revisions
//! are read from [`VcsProvider::status`] rather than by shelling out.

use std::path::Path;

use roder_api::review::{ReviewRequest, ReviewTarget};
use roder_api::version_control::{VcsBase, VcsProvider, VcsStatusRequest};

const UNCOMMITTED_PROMPT: &str = "Review the current code changes (staged, unstaged, and untracked files) and provide prioritized findings.";

/// Used when the workspace's version control provider already knows the merge
/// base for the requested branch, so the reviewer does not have to derive it.
fn base_branch_prompt(branch: &str, merge_base: &str) -> String {
    format!(
        "Review the code changes against the base branch '{branch}'. The merge base commit for this comparison is {merge_base}. Run `git diff {merge_base}` to inspect the changes relative to {branch}. Provide prioritized, actionable findings."
    )
}

/// Used when the merge base is unknown; the reviewer derives it itself.
fn base_branch_prompt_without_merge_base(branch: &str) -> String {
    format!(
        "Review the code changes against the base branch '{branch}'. Start by finding the merge diff between the current branch and {branch}'s upstream e.g. (`git merge-base HEAD \"$(git rev-parse --abbrev-ref \"{branch}@{{upstream}}\")\"`), then run `git diff` against that SHA to see what changes we would merge into the {branch} branch. Provide prioritized, actionable findings."
    )
}

fn commit_prompt(sha: &str, title: Option<&str>) -> String {
    match title {
        Some(title) => format!(
            "Review the code changes introduced by commit {sha} (\"{title}\"). Provide prioritized, actionable findings."
        ),
        None => format!(
            "Review the code changes introduced by commit {sha}. Provide prioritized, actionable findings."
        ),
    }
}

/// Resolves `target` against the workspace's version control state.
///
/// Version control failures are non-fatal: the prompt degrades to the variant
/// that asks the reviewer to derive the diff itself, and `base_sha`/`head_sha`
/// are left unset so publishers can report an unanchored review instead of
/// anchoring to the wrong revision.
pub async fn resolve_review_request(
    target: &ReviewTarget,
    workspace_root: &Path,
    vcs: &dyn VcsProvider,
) -> anyhow::Result<ReviewRequest> {
    let status = vcs
        .status(VcsStatusRequest {
            workspace_root: workspace_root.to_path_buf(),
        })
        .await
        .ok();
    let root = status
        .as_ref()
        .map(|status| status.workspace.root.clone())
        .unwrap_or_else(|| workspace_root.to_path_buf());
    // `roder-ext-git` reports `HEAD` as the workspace id.
    let head = status
        .as_ref()
        .and_then(|status| status.workspace.id.clone());
    let base = status.as_ref().and_then(|status| status.base.clone());

    let (body, base_sha, head_sha) = match target {
        ReviewTarget::UncommittedChanges => (
            UNCOMMITTED_PROMPT.to_string(),
            base.and_then(|base| base.sha),
            head,
        ),
        ReviewTarget::BaseBranch { branch } => match merge_base_for_branch(base.as_ref(), branch) {
            Some(merge_base) => (
                base_branch_prompt(branch, &merge_base),
                Some(merge_base),
                head,
            ),
            None => (base_branch_prompt_without_merge_base(branch), None, head),
        },
        ReviewTarget::Commit { sha, title } => (
            commit_prompt(sha, title.as_deref()),
            None,
            Some(sha.clone()),
        ),
        ReviewTarget::Custom { instructions } => {
            let instructions = instructions.trim();
            anyhow::ensure!(!instructions.is_empty(), "review prompt cannot be empty");
            (
                instructions.to_string(),
                base.and_then(|base| base.sha),
                head,
            )
        }
    };

    let prompt = format!(
        "{body}\n\nThe repository root is {}. Report every finding with an absolute file path under that root.",
        root.display()
    );

    Ok(ReviewRequest {
        target: target.clone(),
        label: review_label(target),
        prompt,
        base_sha,
        head_sha,
        workspace_root: root,
    })
}

/// Short human-readable description of what is being reviewed.
pub fn review_label(target: &ReviewTarget) -> String {
    match target {
        ReviewTarget::UncommittedChanges => "current changes".to_string(),
        ReviewTarget::BaseBranch { branch } => format!("changes against '{branch}'"),
        ReviewTarget::Commit { sha, title } => {
            let short_sha: String = sha.chars().take(7).collect();
            match title {
                Some(title) => format!("commit {short_sha}: {title}"),
                None => format!("commit {short_sha}"),
            }
        }
        ReviewTarget::Custom { instructions } => instructions.trim().to_string(),
    }
}

/// The provider only tracks one base per workspace, so its merge base is only
/// usable when it actually refers to the requested branch. `origin/main` and
/// `main` are treated as the same branch.
fn merge_base_for_branch(base: Option<&VcsBase>, branch: &str) -> Option<String> {
    let base = base?;
    let sha = base.sha.as_deref()?;
    let ref_name = base.ref_name.as_deref()?;
    let matches = ref_name == branch
        || ref_name.ends_with(&format!("/{branch}"))
        || branch.ends_with(&format!("/{ref_name}"));
    matches.then(|| sha.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::review::test_vcs::StubVcs;

    use super::*;

    fn stub(ref_name: Option<&str>, sha: Option<&str>) -> StubVcs {
        StubVcs::new(ref_name, sha)
    }

    #[tokio::test]
    async fn uncommitted_target_carries_base_and_head() {
        let request = resolve_review_request(
            &ReviewTarget::UncommittedChanges,
            &PathBuf::from("/repo"),
            &stub(Some("origin/main"), Some("basesha")),
        )
        .await
        .expect("resolve");

        assert_eq!(request.label, "current changes");
        assert!(
            request
                .prompt
                .starts_with("Review the current code changes")
        );
        assert!(request.prompt.contains("The repository root is /repo"));
        assert_eq!(request.base_sha.as_deref(), Some("basesha"));
        assert_eq!(request.head_sha.as_deref(), Some("headsha"));
    }

    #[tokio::test]
    async fn base_branch_uses_known_merge_base() {
        let request = resolve_review_request(
            &ReviewTarget::BaseBranch {
                branch: "main".to_string(),
            },
            &PathBuf::from("/repo"),
            &stub(Some("origin/main"), Some("mergebase")),
        )
        .await
        .expect("resolve");

        assert_eq!(request.label, "changes against 'main'");
        assert!(request.prompt.contains("git diff mergebase"));
        assert_eq!(request.base_sha.as_deref(), Some("mergebase"));
    }

    #[tokio::test]
    async fn base_branch_falls_back_when_merge_base_is_for_another_branch() {
        let request = resolve_review_request(
            &ReviewTarget::BaseBranch {
                branch: "release".to_string(),
            },
            &PathBuf::from("/repo"),
            &stub(Some("origin/main"), Some("mergebase")),
        )
        .await
        .expect("resolve");

        assert!(request.prompt.contains("git merge-base HEAD"));
        assert!(request.prompt.contains("release@{upstream}"));
        assert!(request.base_sha.is_none());
    }

    #[tokio::test]
    async fn commit_target_labels_short_sha_and_anchors_head() {
        let request = resolve_review_request(
            &ReviewTarget::Commit {
                sha: "0123456789abcdef".to_string(),
                title: Some("Fix the thing".to_string()),
            },
            &PathBuf::from("/repo"),
            &stub(Some("origin/main"), Some("mergebase")),
        )
        .await
        .expect("resolve");

        assert_eq!(request.label, "commit 0123456: Fix the thing");
        assert!(request.prompt.contains("\"Fix the thing\""));
        assert_eq!(request.head_sha.as_deref(), Some("0123456789abcdef"));
        assert!(request.base_sha.is_none());
    }

    #[tokio::test]
    async fn empty_custom_instructions_are_rejected() {
        let error = resolve_review_request(
            &ReviewTarget::Custom {
                instructions: "   ".to_string(),
            },
            &PathBuf::from("/repo"),
            &stub(None, None),
        )
        .await
        .expect_err("empty instructions must fail");

        assert!(error.to_string().contains("cannot be empty"));
    }
}
