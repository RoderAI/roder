use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::discovery::DiscoveryItemId;
use crate::events::{ThreadId, TurnId};

pub type RetrievalRouteId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMode {
    ExactText,
    FileName,
    SemanticCode,
    Artifact,
    History,
    Discovery,
    Promotion,
    Web,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalIntent {
    FindDefinition,
    TraceUsage,
    DebugFailure,
    InspectTool,
    RecoverHistory,
    BroadConcept,
    FileLookup,
    ArtifactInspection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalOutcomeKind {
    Useful,
    Irrelevant,
    StaleIndex,
    MissingIndex,
    AuthRequired,
    MissingPromotion,
    WrongToolFamily,
    WrongMcpServer,
    UnknownTool,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalRecommendation {
    pub mode: RetrievalMode,
    pub tool: String,
    pub query: String,
    pub reason: String,
    pub confidence: RetrievalConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<DiscoveryItemId>,
}

impl RetrievalRecommendation {
    pub fn exact_text(
        tool: impl Into<String>,
        query: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            mode: RetrievalMode::ExactText,
            tool: tool.into(),
            query: query.into(),
            reason: reason.into(),
            confidence: RetrievalConfidence::High,
            item_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalAvoidance {
    pub mode: RetrievalMode,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalRoutePlan {
    pub route_id: RetrievalRouteId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub intent: RetrievalIntent,
    #[serde(default)]
    pub recommended: Vec<RetrievalRecommendation>,
    #[serde(default)]
    pub avoid: Vec<RetrievalAvoidance>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

impl RetrievalRoutePlan {
    pub fn recommended_modes(&self) -> Vec<RetrievalMode> {
        self.recommended
            .iter()
            .map(|recommendation| recommendation.mode.clone())
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalMeasuredOutcome {
    pub route_id: RetrievalRouteId,
    pub mode: RetrievalMode,
    pub tool: String,
    pub outcome: RetrievalOutcomeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_useful_path: Option<RetrievalMode>,
    #[serde(default)]
    pub discovery_before_tool_use: bool,
    #[serde(default)]
    pub promotion_before_tool_use: bool,
    #[serde(default)]
    pub wrong_tool_family_attempts: u64,
    #[serde(default)]
    pub result_count: u64,
    #[serde(default)]
    pub latency_ms: u64,
    #[serde(default)]
    pub bytes_returned: u64,
    #[serde(default)]
    pub estimated_tokens_returned: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalRoutePlanned {
    pub plan: RetrievalRoutePlan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalRouteAccepted {
    pub route_id: RetrievalRouteId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub mode: RetrievalMode,
    pub tool: String,
    pub query: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalRouteIgnored {
    pub route_id: RetrievalRouteId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub chosen_tool: String,
    pub recommended_modes: Vec<RetrievalMode>,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalRouteFailed {
    pub route_id: RetrievalRouteId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub mode: RetrievalMode,
    pub tool: String,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalResultUsed {
    pub outcome: RetrievalMeasuredOutcome,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalDiscoveryItemPromoted {
    pub route_id: RetrievalRouteId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub item_id: DiscoveryItemId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalPromotionSkipped {
    pub route_id: RetrievalRouteId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub item_id: DiscoveryItemId,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
    }

    #[test]
    fn retrieval_route_plan_serializes_recommendations_and_avoidance() {
        let plan = RetrievalRoutePlan {
            route_id: "route-1".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            intent: RetrievalIntent::FindDefinition,
            recommended: vec![
                RetrievalRecommendation::exact_text("grep", "ToolExecutionContext", "exact symbol"),
                RetrievalRecommendation {
                    mode: RetrievalMode::SemanticCode,
                    tool: "code_index.search".to_string(),
                    query: "tool execution policy gate".to_string(),
                    reason: "conceptual fallback".to_string(),
                    confidence: RetrievalConfidence::Medium,
                    item_id: None,
                },
            ],
            avoid: vec![RetrievalAvoidance {
                mode: RetrievalMode::Web,
                reason: "local codebase question".to_string(),
            }],
            timestamp: timestamp(),
        };

        let value = serde_json::to_value(&plan).unwrap();
        assert_eq!(value["intent"], "find_definition");
        assert_eq!(value["recommended"][0]["mode"], "exact_text");
        assert_eq!(value["recommended"][1]["tool"], "code_index.search");
        assert_eq!(value["avoid"][0]["mode"], "web");

        let round_trip: RetrievalRoutePlan = serde_json::from_value(value).unwrap();
        assert_eq!(
            round_trip.recommended_modes(),
            vec![RetrievalMode::ExactText, RetrievalMode::SemanticCode]
        );
    }

    #[test]
    fn retrieval_outcome_tracks_discovery_promotion_and_noise() {
        let outcome = RetrievalMeasuredOutcome {
            route_id: "route-2".to_string(),
            mode: RetrievalMode::Discovery,
            tool: "discovery.search".to_string(),
            outcome: RetrievalOutcomeKind::Useful,
            first_useful_path: Some(RetrievalMode::Discovery),
            discovery_before_tool_use: true,
            promotion_before_tool_use: true,
            wrong_tool_family_attempts: 0,
            result_count: 1,
            latency_ms: 12,
            bytes_returned: 512,
            estimated_tokens_returned: 128,
        };

        let value = serde_json::to_value(&outcome).unwrap();
        assert_eq!(value["firstUsefulPath"], "discovery");
        assert_eq!(value["promotionBeforeToolUse"], true);
        assert_eq!(value["wrongToolFamilyAttempts"], 0);
    }

    #[test]
    fn retrieval_events_use_camel_case_fields() {
        let event = RetrievalPromotionSkipped {
            route_id: "route-3".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_id: "mcp:github/issues.search".to_string(),
            reason: "auth required".to_string(),
            timestamp: timestamp(),
        };

        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["routeId"], "route-3");
        assert_eq!(value["itemId"], "mcp:github/issues.search");
    }
}
