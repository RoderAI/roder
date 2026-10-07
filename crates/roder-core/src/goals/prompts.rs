use roder_api::goals::ThreadGoal;
use roder_api::policy_mode::PolicyMode;

pub(super) fn permission_prompt(mode: PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Bypass => include_str!("full_access.md"),
        PolicyMode::Plan => {
            "Goal permissions: Plan mode. Inspect and plan within the current permissions; file modifications remain denied. A goal does not grant permission to leave Plan mode."
        }
        PolicyMode::AcceptAll => {
            "Goal permissions: Accept All mode. Continue authorized work using the configured auto-approved tools. Other tools may require approval. A goal does not expand these permissions."
        }
        PolicyMode::Default => {
            "Goal permissions: Default mode. Continue authorized work; side-effecting tools use the normal approval flow. A goal does not grant Full Access or expand these permissions."
        }
    }
}

// Semantics from openai/codex 87be737b664: ext/goal/templates/goals.
fn render(template: &str, goal: &ThreadGoal) -> String {
    // Replace the objective last so user text cannot introduce template substitutions.
    template
        .replace("{{ tokens_used }}", &goal.tokens_used.to_string())
        .replace(
            "{{ time_used_seconds }}",
            &goal.time_used_seconds.to_string(),
        )
        .replace(
            "{{ token_budget }}",
            &goal
                .token_budget
                .map(|b| b.to_string())
                .unwrap_or_else(|| "none".into()),
        )
        .replace(
            "{{ remaining_tokens }}",
            &goal
                .token_budget
                .map(|b| b.saturating_sub(goal.tokens_used).max(0).to_string())
                .unwrap_or_else(|| "unbounded".into()),
        )
        .replace(
            "{{ objective }}",
            &goal
                .objective
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;"),
        )
}

pub(crate) fn continuation_prompt(goal: &ThreadGoal) -> String {
    render(include_str!("continuation.md"), goal)
}

pub(super) fn objective_updated_prompt(goal: &ThreadGoal) -> String {
    render(include_str!("objective_updated.md"), goal)
}

pub(super) fn budget_limit_prompt(goal: &ThreadGoal) -> String {
    render(include_str!("budget_limit.md"), goal)
}
