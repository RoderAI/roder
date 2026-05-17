use roder_api::teams::{TeamMember, TeamMemberId, TeamSnapshot};

const BROADCAST_TERMS: [&str; 7] = [
    "everyone",
    "everybody",
    "all",
    "team",
    "guys",
    "folks",
    "yall",
];

pub(crate) fn responders_for_channel_message(
    team: &TeamSnapshot,
    channel_id: &str,
    text: &str,
) -> Vec<TeamMemberId> {
    let mentioned = mentioned_members(team, text);
    if !mentioned.is_empty() {
        return mentioned;
    }

    if is_broadcast(text) {
        return team
            .members
            .iter()
            .map(|member| member.id.clone())
            .collect();
    }

    let roles = default_channel_roles(channel_id);
    let mut selected = roles
        .iter()
        .filter_map(|role| {
            team.members
                .iter()
                .find(|member| member.role == *role)
                .map(|member| member.id.clone())
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        selected.extend(team.members.iter().take(3).map(|member| member.id.clone()));
    }
    selected
}

pub(crate) fn team_channel_reply_prompt(
    channel_id: &str,
    message_text: &str,
    member: &TeamMember,
) -> String {
    format!(
        "You are {} in #{}. Reply naturally in the channel to this teammate message. Keep it concise, concrete, and speak only as yourself. Message: {}",
        member.display_name,
        channel_id,
        message_text.trim()
    )
}

fn mentioned_members(team: &TeamSnapshot, text: &str) -> Vec<TeamMemberId> {
    let normalized_text = normalize(text);
    team.members
        .iter()
        .filter(|member| member_is_mentioned(&normalized_text, member))
        .map(|member| member.id.clone())
        .collect()
}

fn member_is_mentioned(normalized_text: &str, member: &TeamMember) -> bool {
    mention_variants(member).iter().any(|variant| {
        normalized_text.contains(&format!("@{variant} "))
            || normalized_text.contains(&format!("@{variant},"))
            || normalized_text.contains(&format!("@{variant}:"))
            || normalized_text.ends_with(&format!("@{variant}"))
    })
}

fn mention_variants(member: &TeamMember) -> Vec<String> {
    let mut variants = vec![
        normalize(&member.id),
        normalize(&member.role),
        normalize(&member.display_name),
    ];
    variants.sort();
    variants.dedup();
    variants
}

fn is_broadcast(text: &str) -> bool {
    let normalized = normalize(text);
    BROADCAST_TERMS
        .iter()
        .any(|term| normalized.split_whitespace().any(|word| word == *term))
}

fn default_channel_roles(channel_id: &str) -> &'static [&'static str] {
    match channel_id {
        "reviews" => &["reviewer", "backend", "qa"],
        "debugging" => &["backend", "infra", "qa"],
        "architecture" => &["engineering-lead", "backend", "infra", "ux"],
        "shipping" => &["release", "qa", "pm"],
        "research" => &["research", "pm", "ux"],
        "ideas" => &["pm", "research", "ux"],
        "standup" | "general" => &["engineering-lead", "pm", "frontend", "backend", "qa"],
        "random" => &["pm", "research", "ux"],
        _ => &["engineering-lead", "pm", "backend"],
    }
}

fn normalize(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '@' {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
