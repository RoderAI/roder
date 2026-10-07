use roder_api::goals::ThreadGoal;

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
