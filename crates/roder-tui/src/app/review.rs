//! `/review` — start a read-only review sub-turn from the composer.
//!
//! The command is pure RPC (the `/compact` shape): it resolves a
//! [`ReviewTarget`] from the raw argument string and issues `review/start` with
//! detached delivery, so the composer stays responsive while the reviewer runs.
//! Findings arrive later as `RoderEvent::ReviewCompleted`, which renders them
//! into the timeline and opens the review panel.

use roder_api::review::{ReviewDelivery, ReviewTarget};
use roder_app_server::AppClient;
use roder_protocol::{JsonRpcRequest, ReviewStartParams, ReviewStartResult};

use super::{TuiApp, decode_response, slash_command_suffix};

/// Everything `/review` accepts, already validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewArgs {
    pub(super) target: ReviewTarget,
    /// `--publish <id>`: the publisher the review panel should use when the
    /// user presses `p`. Publishing is never automatic.
    pub(super) publisher_id: Option<String>,
}

/// Hand-parses the raw argument string (there is no clap in this repo).
///
/// * no arguments — the uncommitted working diff
/// * `--base <branch>` / `--commit <sha>` / `--uncommitted` — an explicit target
/// * `--publish <id>` — preselect a publisher for the panel
/// * anything else — a free-form [`ReviewTarget::Custom`] scope
///
/// Free text alongside an explicit target is rejected rather than silently
/// dropped, and an unknown `--flag` is an error rather than review instructions.
pub(super) fn parse_review_args(args: &str) -> Result<ReviewArgs, String> {
    let tokens = args.split_whitespace().collect::<Vec<_>>();
    let mut target: Option<ReviewTarget> = None;
    let mut publisher_id: Option<String> = None;
    let mut instructions: Vec<&str> = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        match token {
            "--uncommitted" => {
                set_target(&mut target, ReviewTarget::UncommittedChanges)?;
                index += 1;
            }
            "--base" | "--commit" | "--publish" => {
                let value = option_value(&tokens, index)?;
                match token {
                    "--base" => set_target(
                        &mut target,
                        ReviewTarget::BaseBranch {
                            branch: value.to_string(),
                        },
                    )?,
                    "--commit" => set_target(
                        &mut target,
                        ReviewTarget::Commit {
                            sha: value.to_string(),
                            title: None,
                        },
                    )?,
                    _ => {
                        if publisher_id.is_some() {
                            return Err("--publish was given twice".to_string());
                        }
                        publisher_id = Some(value.to_string());
                    }
                }
                index += 2;
            }
            _ if token.starts_with("--") => {
                return Err(format!(
                    "unknown option {token}; expected --base, --commit, --uncommitted, or --publish"
                ));
            }
            _ => {
                instructions.push(token);
                index += 1;
            }
        }
    }

    let target = match (target, instructions.is_empty()) {
        (Some(target), true) => target,
        (Some(_), false) => {
            return Err(format!(
                "unexpected argument '{}'; drop the target flag to review a free-form scope",
                instructions.join(" ")
            ));
        }
        (None, true) => ReviewTarget::UncommittedChanges,
        (None, false) => ReviewTarget::Custom {
            instructions: instructions.join(" "),
        },
    };
    Ok(ReviewArgs {
        target,
        publisher_id,
    })
}

fn set_target(slot: &mut Option<ReviewTarget>, target: ReviewTarget) -> Result<(), String> {
    if slot.is_some() {
        return Err("only one review target may be given".to_string());
    }
    *slot = Some(target);
    Ok(())
}

fn option_value<'a>(tokens: &[&'a str], index: usize) -> Result<&'a str, String> {
    tokens
        .get(index + 1)
        .copied()
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| format!("{} needs a value", tokens[index]))
}

/// One-line description of what is being reviewed, for the timeline.
pub(super) fn target_label(target: &ReviewTarget) -> String {
    match target {
        ReviewTarget::UncommittedChanges => "uncommitted changes".to_string(),
        ReviewTarget::BaseBranch { branch } => format!("changes vs {branch}"),
        ReviewTarget::Commit { sha, .. } => format!("commit {sha}"),
        ReviewTarget::Custom { instructions } => instructions.clone(),
    }
}

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn run_review_slash_command(&mut self, args: &str) {
        let parsed = match parse_review_args(args) {
            Ok(parsed) => parsed,
            Err(message) => {
                self.record_error(format!("/review: {message}"));
                return;
            }
        };
        let label = target_label(&parsed.target);
        let params = ReviewStartParams {
            thread_id: self.thread_id.clone(),
            target: parsed.target,
            // Detached: the reviewer can run for minutes and the composer must
            // stay usable. Findings arrive as `review/completed`.
            delivery: ReviewDelivery::Detached,
            publish: None,
        };
        match review_start(&self.client, params).await {
            Ok(result) => {
                self.review_publisher = parsed.publisher_id;
                self.timeline
                    .push_system(format!("Reviewing {label}. Findings will open in a panel."));
                self.push_event(format!("review started: {}", result.review_id));
            }
            Err(err) => self.record_error(format!("review/start failed: {err}")),
        }
        self.push_event(format!(
            "slash command: /review{}",
            slash_command_suffix(args)
        ));
    }
}

async fn review_start<C: AppClient>(
    client: &C,
    params: ReviewStartParams,
) -> anyhow::Result<ReviewStartResult> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("review/start")),
            method: "review/start".to_string(),
            params: Some(serde_json::to_value(params)?),
        })
        .await;
    decode_response(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &str) -> ReviewArgs {
        parse_review_args(args).expect("args should parse")
    }

    #[test]
    fn no_arguments_reviews_the_working_diff() {
        assert_eq!(
            parse(""),
            ReviewArgs {
                target: ReviewTarget::UncommittedChanges,
                publisher_id: None,
            }
        );
        assert_eq!(parse("   ").target, ReviewTarget::UncommittedChanges);
    }

    #[test]
    fn explicit_targets_are_recognised() {
        assert_eq!(
            parse("--base main").target,
            ReviewTarget::BaseBranch {
                branch: "main".to_string()
            }
        );
        assert_eq!(
            parse("--commit deadbeef").target,
            ReviewTarget::Commit {
                sha: "deadbeef".to_string(),
                title: None
            }
        );
        assert_eq!(
            parse("--uncommitted").target,
            ReviewTarget::UncommittedChanges
        );
    }

    #[test]
    fn free_text_becomes_a_custom_scope() {
        assert_eq!(
            parse("check the auth middleware").target,
            ReviewTarget::Custom {
                instructions: "check the auth middleware".to_string()
            }
        );
    }

    #[test]
    fn publish_flag_preselects_a_publisher_without_changing_the_target() {
        let parsed = parse("--publish github check the parser");
        assert_eq!(parsed.publisher_id.as_deref(), Some("github"));
        assert_eq!(
            parsed.target,
            ReviewTarget::Custom {
                instructions: "check the parser".to_string()
            }
        );
    }

    #[test]
    fn malformed_arguments_are_rejected_rather_than_silently_dropped() {
        assert!(parse_review_args("--base").is_err());
        assert!(parse_review_args("--base --commit abc").is_err());
        assert!(parse_review_args("--base main --commit abc").is_err());
        assert!(parse_review_args("--base main also look at x").is_err());
        assert!(parse_review_args("--bogus").is_err());
    }

    #[test]
    fn labels_describe_the_target() {
        assert_eq!(
            target_label(&ReviewTarget::UncommittedChanges),
            "uncommitted changes"
        );
        assert_eq!(
            target_label(&ReviewTarget::BaseBranch {
                branch: "main".to_string()
            }),
            "changes vs main"
        );
    }
}
