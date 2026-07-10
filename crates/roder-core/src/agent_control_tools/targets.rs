use roder_api::events::ThreadId;
use roder_api::teams::{TeamId, TeamMemberDescriptor};

use crate::runtime::Runtime;
use crate::teams::TeamState;

#[derive(Debug, Clone)]
pub(super) struct AgentTarget {
    pub(super) team_id: TeamId,
    pub(super) member: TeamMemberDescriptor,
}

impl Runtime {
    pub(super) async fn caller_agents(&self, parent_thread_id: &ThreadId) -> Vec<TeamState> {
        self.list_teams()
            .await
            .into_iter()
            .filter(|team| {
                team.lead_thread_id == *parent_thread_id
                    || team
                        .members
                        .iter()
                        .any(|member| member.thread_id == *parent_thread_id)
            })
            .collect()
    }

    pub(super) async fn resolve_agent_target(
        &self,
        parent_thread_id: &ThreadId,
        target: &str,
    ) -> anyhow::Result<AgentTarget> {
        let target = target.trim();
        anyhow::ensure!(!target.is_empty(), "agent target must not be empty");
        let teams = self.caller_agents(parent_thread_id).await;
        anyhow::ensure!(!teams.is_empty(), "caller is not part of an agent team");
        let caller_path = teams
            .iter()
            .flat_map(|team| team.members.iter())
            .find(|member| member.thread_id == *parent_thread_id)
            .and_then(member_agent_path)
            .unwrap_or_else(|| "/root".to_string());
        let relative_path = canonical_agent_path(&caller_path, target);
        let all_targets = teams
            .into_iter()
            .flat_map(|team| {
                let team_id = team.id;
                team.members.into_iter().map(move |member| AgentTarget {
                    team_id: team_id.clone(),
                    member,
                })
            })
            .collect::<Vec<_>>();

        let by_stable_id = all_targets
            .iter()
            .filter(|candidate| {
                candidate.member.id == target || candidate.member.thread_id == target
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Some(target) = unique_target(target, by_stable_id)? {
            return Ok(target);
        }

        let by_path = all_targets
            .iter()
            .filter(|candidate| {
                member_agent_path(&candidate.member).as_deref() == Some(relative_path.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Some(target) = unique_target(target, by_path)? {
            return Ok(target);
        }

        if !target.contains('/') {
            let by_task_name = all_targets
                .into_iter()
                .filter(|candidate| {
                    candidate.member.task_name.as_deref() == Some(target)
                        || candidate.member.name == target
                        || candidate.member.name.split(':').next() == Some(target)
                })
                .collect::<Vec<_>>();
            if let Some(target) = unique_target(target, by_task_name)? {
                return Ok(target);
            }
        }

        anyhow::bail!("unknown agent target {target:?}")
    }
}

pub(super) fn member_agent_path(member: &TeamMemberDescriptor) -> Option<String> {
    member.agent_path.clone().or_else(|| {
        (member.role == roder_api::teams::TeamMemberRole::Lead).then(|| "/root".to_string())
    })
}

pub(super) fn member_identity(member: &TeamMemberDescriptor) -> &str {
    member
        .agent_path
        .as_deref()
        .or(member.task_name.as_deref())
        .unwrap_or(&member.name)
}

pub(super) fn canonical_agent_path(caller_path: &str, target: &str) -> String {
    let target = target.trim();
    if target.starts_with('/') {
        let normalized = target.trim_end_matches('/');
        return if normalized.is_empty() {
            "/".to_string()
        } else {
            normalized.to_string()
        };
    }
    format!(
        "{}/{}",
        caller_path.trim_end_matches('/'),
        target.trim_matches('/')
    )
}

pub(super) fn agent_path_matches_prefix(agent_path: &str, prefix: &str) -> bool {
    agent_path == prefix
        || agent_path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn unique_target(target: &str, matches: Vec<AgentTarget>) -> anyhow::Result<Option<AgentTarget>> {
    match matches.as_slice() {
        [] => Ok(None),
        [matched] => Ok(Some(matched.clone())),
        _ => anyhow::bail!(
            "ambiguous agent target {target:?}; use the canonical path from list_agents"
        ),
    }
}

pub(super) fn reject_root_or_self(
    caller_thread_id: &ThreadId,
    target: &TeamMemberDescriptor,
    action: &str,
) -> Result<(), String> {
    if target.role == roder_api::teams::TeamMemberRole::Lead
        || target.agent_path.as_deref() == Some("/root")
    {
        return Err("root is not a spawned agent".to_string());
    }
    if target.thread_id == *caller_thread_id {
        return Err(format!("an agent cannot {action} itself"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_and_prefixes_preserve_task_tree_boundaries() {
        assert_eq!(
            canonical_agent_path("/root/task1", "child"),
            "/root/task1/child"
        );
        assert_eq!(
            canonical_agent_path("/root/task1", "/root/task2/child"),
            "/root/task2/child"
        );
        assert!(agent_path_matches_prefix(
            "/root/task1/child",
            "/root/task1"
        ));
        assert!(!agent_path_matches_prefix("/root/task10", "/root/task1"));
    }
}
