//! Decode the upstream context code inside OpenRouter's numeric HTTP envelope.
use reqwest::StatusCode;
use serde_json::Value;

pub(super) fn context_code(status: StatusCode, response: &Value) -> Option<&'static str> {
    if status != StatusCode::BAD_REQUEST {
        return None;
    }
    let raw = response.pointer("/error/metadata/raw")?.as_str()?;
    let upstream: Value = serde_json::from_str(raw).ok()?;
    let error = upstream.get("error")?;
    ["code", "type"]
        .into_iter()
        .find_map(|field| match error.get(field).and_then(Value::as_str) {
            Some("context_length_exceeded") => Some("context_length_exceeded"),
            Some("context_window_exceeded") => Some("context_window_exceeded"),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(raw: Value) -> Value {
        json!({"error": {"code": 400, "message": "Provider returned error",
            "metadata": {"raw": raw}}})
    }

    #[test]
    fn accepts_only_structured_upstream_context_codes() {
        for field in ["code", "type"] {
            for code in ["context_length_exceeded", "context_window_exceeded"] {
                let body = response(json!(json!({"error": {field: code}}).to_string()));
                assert_eq!(context_code(StatusCode::BAD_REQUEST, &body), Some(code));
            }
        }
    }

    #[test]
    fn malformed_or_unrelated_upstream_errors_stay_unclassified() {
        for raw in [
            json!("context_length_exceeded"),
            json!(r#"{"error":{"message":"context_length_exceeded"}}"#),
            json!(
                r#"{"error":{"code":"invalid_request_error","message":"maximum context length is a field name"}}"#
            ),
            json!(r#"{"error":{"code":"context_length_exceeded"}"#),
            json!({"error": {"code": "context_length_exceeded"}}),
            Value::Null,
        ] {
            assert_eq!(context_code(StatusCode::BAD_REQUEST, &response(raw)), None);
        }
    }

    #[test]
    fn other_http_statuses_keep_their_own_recovery_policy() {
        let body = response(json!(r#"{"error":{"code":"context_length_exceeded"}}"#));
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::PAYMENT_REQUIRED,
            StatusCode::FORBIDDEN,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert_eq!(context_code(status, &body), None);
        }
    }
}
