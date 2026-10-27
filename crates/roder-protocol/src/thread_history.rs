use serde::{Deserialize, Serialize};

use crate::Thread;

/// Read a bounded chronological page of transcript items, starting at the tail.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadHistoryReadParams {
    pub thread_id: String,
    /// Exclusive opaque anchor returned by the preceding page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    /// Item limit, not a turn limit (default 160 initially, 120 for older pages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Serialized item byte budget; a single oversized item still makes progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadHistoryReadResult {
    pub thread: Option<Thread>,
    pub before: Option<String>,
    pub has_more: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn history_request_uses_camel_case_and_optional_budgets() {
        let params: ThreadHistoryReadParams =
            serde_json::from_value(json!({"threadId": "t"})).unwrap();
        assert!(params.before.is_none());
        assert!(params.limit.is_none());
        assert!(params.max_bytes.is_none());
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            json!({"threadId": "t"})
        );
        let params: ThreadHistoryReadParams = serde_json::from_value(json!({
            "threadId": "t", "before": "opaque", "limit": 7, "maxBytes": 99
        }))
        .unwrap();
        assert_eq!(params.before.as_deref(), Some("opaque"));
        assert_eq!(params.limit, Some(7));
        assert_eq!(params.max_bytes, Some(99));
    }

    #[test]
    fn history_request_rejects_mistyped_or_unknown_paging_fields() {
        for value in [
            json!({"threadId":"t", "limit": -1}),
            json!({"threadId":"t", "before": 42}),
            json!({"threadId":"t", "max_bytes": 9}),
        ] {
            assert!(serde_json::from_value::<ThreadHistoryReadParams>(value).is_err());
        }
    }

    #[test]
    fn history_unknown_thread_has_explicit_empty_page() {
        let result = ThreadHistoryReadResult {
            thread: None,
            before: None,
            has_more: false,
        };
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({"thread": null, "before": null, "hasMore": false})
        );
    }

    #[test]
    fn history_manifest_names_the_canonical_bounded_contract() {
        let spec = crate::methods::app_server_method_specs()
            .into_iter()
            .find(|spec| spec.method == "thread/history/read")
            .unwrap();
        assert_eq!(spec.params_type, "ThreadHistoryReadParams");
        assert_eq!(spec.result_type, "ThreadHistoryReadResult");
        assert_eq!(
            spec.side_effect,
            crate::methods::AppServerSideEffect::ReadOnly
        );
    }
}
