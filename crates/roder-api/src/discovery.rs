use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};

pub type DiscoveryCatalogId = String;
pub type DiscoveryGroupId = String;
pub type DiscoveryItemId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum DiscoverySourceKind {
    InternalTools,
    McpTools,
    Skills,
    Commands,
    Plugins,
    Subagents,
    ArtifactTools,
    WorkflowImports,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiscoveryItemStatus {
    Available,
    Disabled,
    Unavailable,
    AuthRequired,
    Error,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiscoveryAuthState {
    NotRequired,
    Available,
    Required,
    Expired,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiscoveryLifecycleState {
    Discovered,
    Promoted,
    Reused,
    WarmCached,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiscoveryPromotionState {
    NotPromoted,
    Promoted,
    Reused,
    WarmCacheHit,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiscoveryCacheStatus {
    Cold,
    Warm,
    Hit,
    Expired,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiscoverySchemaFormat {
    JsonSchema,
    Markdown,
    Toml,
    Json,
    PlainText,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryRedaction {
    #[serde(default)]
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_refs: Vec<String>,
}

impl DiscoveryRedaction {
    pub fn none() -> Self {
        Self {
            redacted: false,
            fields: Vec::new(),
            secret_refs: Vec::new(),
        }
    }

    pub fn secret_refs(secret_refs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            redacted: true,
            fields: Vec::new(),
            secret_refs: secret_refs.into_iter().map(Into::into).collect(),
        }
    }
}

impl Default for DiscoveryRedaction {
    fn default() -> Self {
        Self::none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverySchemaReference {
    pub format: DiscoverySchemaFormat,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_count: Option<u64>,
    #[serde(default)]
    pub redaction: DiscoveryRedaction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCatalogSource {
    pub kind: DiscoverySourceKind,
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default)]
    pub auth_state: DiscoveryAuthState,
    #[serde(default)]
    pub redaction: DiscoveryRedaction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCatalogItem {
    pub id: DiscoveryItemId,
    pub group_id: DiscoveryGroupId,
    pub source: DiscoveryCatalogSource,
    pub name: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: DiscoveryItemStatus,
    pub lifecycle: DiscoveryLifecycleState,
    pub promotion: DiscoveryPromotionState,
    pub cache_status: DiscoveryCacheStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<DiscoverySchemaReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    #[serde(default)]
    pub redaction: DiscoveryRedaction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_refreshed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCatalogGroup {
    pub id: DiscoveryGroupId,
    pub catalog_id: DiscoveryCatalogId,
    pub source: DiscoveryCatalogSource,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: DiscoveryItemStatus,
    #[serde(default)]
    pub item_count: u64,
    #[serde(default)]
    pub hidden_item_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DiscoveryCatalogItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_refreshed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCatalog {
    pub id: DiscoveryCatalogId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<DiscoveryCatalogGroup>,
    #[serde(default)]
    pub hidden_item_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::rfc3339::option")]
    pub built_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPromotionRecord {
    pub item_id: DiscoveryItemId,
    pub group_id: DiscoveryGroupId,
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub promotion: DiscoveryPromotionState,
    pub cache_status: DiscoveryCacheStatus,
    #[serde(default)]
    pub reused_count: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCatalogBuilt {
    pub catalog_id: DiscoveryCatalogId,
    pub group_count: u64,
    pub item_count: u64,
    pub hidden_item_count: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryItemUpdated {
    pub item: DiscoveryCatalogItem,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryAuthRequired {
    pub item_id: DiscoveryItemId,
    pub group_id: DiscoveryGroupId,
    pub source: DiscoveryCatalogSource,
    pub auth_state: DiscoveryAuthState,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryItemRead {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub item_id: DiscoveryItemId,
    pub group_id: DiscoveryGroupId,
    pub promoted: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryItemPromoted {
    pub record: DiscoveryPromotionRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPromotionReused {
    pub record: DiscoveryPromotionRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryWarmCacheHit {
    pub record: DiscoveryPromotionRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPromotionExpired {
    pub record: DiscoveryPromotionRecord,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(kind: DiscoverySourceKind, id: &str) -> DiscoveryCatalogSource {
        DiscoveryCatalogSource {
            kind,
            id: id.to_string(),
            display_name: id.to_string(),
            origin: Some(format!("fixture://{id}")),
            auth_state: DiscoveryAuthState::NotRequired,
            redaction: DiscoveryRedaction::none(),
        }
    }

    fn item(kind: DiscoverySourceKind, id: &str) -> DiscoveryCatalogItem {
        DiscoveryCatalogItem {
            id: id.to_string(),
            group_id: format!("group-{id}"),
            source: source(kind, id),
            name: id.to_string(),
            title: id.to_string(),
            description: Some(format!("{id} description")),
            status: DiscoveryItemStatus::Available,
            lifecycle: DiscoveryLifecycleState::Discovered,
            promotion: DiscoveryPromotionState::NotPromoted,
            cache_status: DiscoveryCacheStatus::Cold,
            schema: None,
            tags: vec!["fixture".to_string()],
            hints: vec!["read before use".to_string()],
            redaction: DiscoveryRedaction::none(),
            last_refreshed_at: None,
        }
    }

    #[test]
    fn discovery_items_round_trip_for_all_source_families() {
        let entries = vec![
            item(DiscoverySourceKind::InternalTools, "tool:grep"),
            item(DiscoverySourceKind::McpTools, "mcp:github/issues.search"),
            item(DiscoverySourceKind::Skills, "skill:rust-clippy"),
            item(DiscoverySourceKind::Commands, "command:project/test"),
            item(DiscoverySourceKind::Plugins, "plugin:codex/review"),
        ];

        let value = serde_json::to_value(&entries).expect("serialize discovery entries");
        let decoded: Vec<DiscoveryCatalogItem> =
            serde_json::from_value(value).expect("deserialize discovery entries");
        assert_eq!(decoded, entries);
    }

    #[test]
    fn mcp_auth_metadata_redacts_secret_values() {
        let entry = DiscoveryCatalogItem {
            source: DiscoveryCatalogSource {
                kind: DiscoverySourceKind::McpTools,
                id: "github".to_string(),
                display_name: "GitHub MCP".to_string(),
                origin: Some("mcp://github".to_string()),
                auth_state: DiscoveryAuthState::Required,
                redaction: DiscoveryRedaction::secret_refs(["GITHUB_TOKEN"]),
            },
            status: DiscoveryItemStatus::AuthRequired,
            redaction: DiscoveryRedaction {
                redacted: true,
                fields: vec!["env.GITHUB_TOKEN".to_string()],
                secret_refs: vec!["GITHUB_TOKEN".to_string()],
            },
            ..item(DiscoverySourceKind::McpTools, "mcp:github/issues.search")
        };

        let value = serde_json::to_value(&entry).expect("serialize redacted entry");
        let rendered = value.to_string();
        assert!(rendered.contains("GITHUB_TOKEN"));
        assert!(!rendered.contains("ghp_"));
        assert_eq!(value["authState"], serde_json::Value::Null);
        assert_eq!(value["source"]["authState"], "required");
        assert_eq!(value["redaction"]["redacted"], true);
    }

    #[test]
    fn schema_reference_uses_camel_case_fields() {
        let mut entry = item(DiscoverySourceKind::InternalTools, "tool:grep");
        entry.schema = Some(DiscoverySchemaReference {
            format: DiscoverySchemaFormat::JsonSchema,
            uri: "discovery/tools/builtin/grep.schema.json".to_string(),
            content_hash: Some("sha256:abc".to_string()),
            byte_count: Some(512),
            redaction: DiscoveryRedaction::none(),
        });

        let value = serde_json::to_value(&entry).expect("serialize schema entry");
        assert_eq!(value["schema"]["contentHash"], "sha256:abc");
        assert_eq!(value["schema"]["byteCount"], 512);
        let decoded: DiscoveryCatalogItem =
            serde_json::from_value(value).expect("deserialize schema entry");
        assert_eq!(decoded, entry);
    }

    #[test]
    fn promotion_record_tracks_thread_lifecycle() {
        let record = DiscoveryPromotionRecord {
            item_id: "skill:roadmap-planning".to_string(),
            group_id: "skills".to_string(),
            thread_id: "thread-a".to_string(),
            turn_id: Some("turn-b".to_string()),
            promotion: DiscoveryPromotionState::WarmCacheHit,
            cache_status: DiscoveryCacheStatus::Hit,
            reused_count: 3,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        };

        let value = serde_json::to_value(&record).expect("serialize promotion");
        assert_eq!(value["cacheStatus"], "hit");
        assert_eq!(value["reusedCount"], 3);
        let decoded: DiscoveryPromotionRecord =
            serde_json::from_value(value).expect("deserialize promotion");
        assert_eq!(decoded, record);
    }
}
