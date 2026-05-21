use roder_api::context::ContextBlock;
use roder_api::events::{RoderEvent, TurnId};
use roder_api::skills::{SkillIndexRendered, SkillInvoked, SkillSkipped};
use roder_skills::{
    SkillResolutionError, parse_skill_invocations, render_global_skill_index, render_skill_body,
};
use time::OffsetDateTime;

use crate::runtime::{Runtime, StartTurnRequest};

impl Runtime {
    pub(crate) async fn skill_context_blocks(
        &self,
        req: &StartTurnRequest,
        turn_id: &TurnId,
    ) -> Vec<ContextBlock> {
        let skill_registry = self.skills.read().await.clone();
        let mut blocks = Vec::new();
        if let Some(block) = render_global_skill_index(skill_registry.skills()) {
            let rendered_count = skill_registry
                .skills()
                .iter()
                .filter(|skill| {
                    skill.descriptor.activation == roder_api::skills::SkillActivationState::Enabled
                        && skill.descriptor.exposure == roder_api::skills::SkillExposure::Global
                })
                .count() as u64;
            let hidden_count = skill_registry.skills().len() as u64 - rendered_count;
            self.emit(RoderEvent::SkillIndexRendered(SkillIndexRendered {
                thread_id: req.thread_id.clone(),
                turn_id: turn_id.clone(),
                rendered_count,
                hidden_count,
                estimated_tokens: block
                    .token_estimate
                    .unwrap_or_else(|| estimate_text_tokens(&block.text)),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            blocks.push(block);
        }
        for selector in parse_skill_invocations(&req.message) {
            match skill_registry.resolve(&selector) {
                Ok(skill) => {
                    self.emit(RoderEvent::SkillInvoked(SkillInvoked {
                        thread_id: req.thread_id.clone(),
                        turn_id: turn_id.clone(),
                        selector: selector.clone(),
                        descriptor: skill.descriptor.clone(),
                        timestamp: OffsetDateTime::now_utc(),
                    }))
                    .await;
                    blocks.push(render_skill_body(skill));
                }
                Err(err) => {
                    self.emit(RoderEvent::SkillSkipped(SkillSkipped {
                        thread_id: req.thread_id.clone(),
                        turn_id: turn_id.clone(),
                        selector,
                        reason: skill_resolution_error_message(&err),
                        timestamp: OffsetDateTime::now_utc(),
                    }))
                    .await;
                }
            }
        }
        blocks
    }
}

fn skill_resolution_error_message(error: &SkillResolutionError) -> String {
    match error {
        SkillResolutionError::Missing(_) => "skill not found".to_string(),
        SkillResolutionError::Disabled(path) => format!("skill disabled: {path}"),
        SkillResolutionError::Ambiguous {
            name,
            canonical_paths,
        } => format!(
            "skill name {name} is ambiguous; select by canonical path: {}",
            canonical_paths.join(", ")
        ),
    }
}

fn estimate_text_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}
