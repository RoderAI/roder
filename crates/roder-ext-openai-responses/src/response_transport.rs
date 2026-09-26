use super::*;

pub(super) struct RetriedResponse {
    pub(super) response: reqwest::Response,
    pub(super) retry_events: Vec<Value>,
    pub(super) idle_timeout: Duration,
}

pub(super) async fn send_responses_request(
    base_url: &str,
    api_key: &str,
    headers: &[(String, String)],
    grok_conversation_id: Option<&str>,
    body: &Value,
    policy: Option<&ReliabilityRequestPolicy>,
) -> anyhow::Result<RetriedResponse> {
    send_responses_request_with_idle_timeout(
        base_url,
        api_key,
        headers,
        grok_conversation_id,
        body,
        policy,
        responses_stream_idle_timeout(),
    )
    .await
}

pub(super) async fn send_responses_request_with_idle_timeout(
    base_url: &str,
    api_key: &str,
    headers: &[(String, String)],
    grok_conversation_id: Option<&str>,
    body: &Value,
    policy: Option<&ReliabilityRequestPolicy>,
    idle_timeout: Duration,
) -> anyhow::Result<RetriedResponse> {
    send_responses_endpoint(
        base_url,
        api_key,
        headers,
        grok_conversation_id,
        body,
        policy,
        ResponseEndpoint {
            idle_timeout,
            path: "responses",
        },
    )
    .await
}

pub(super) struct ResponseEndpoint {
    pub idle_timeout: Duration,
    pub path: &'static str,
}

pub(super) async fn send_responses_endpoint(
    base_url: &str,
    api_key: &str,
    headers: &[(String, String)],
    grok_conversation_id: Option<&str>,
    body: &Value,
    policy: Option<&ReliabilityRequestPolicy>,
    endpoint: ResponseEndpoint,
) -> anyhow::Result<RetriedResponse> {
    let ResponseEndpoint {
        idle_timeout,
        path: endpoint,
    } = endpoint;
    let policy = policy.cloned().unwrap_or_default();
    let attempts = policy.provider_retry_max_attempts.max(1);
    let payload = prepare_request_payload(body, request_byte_limit())?;
    let client = responses_stream_client(idle_timeout)?;
    let mut last_error = None;
    let mut retry_events = Vec::new();
    for attempt in 1..=attempts {
        let mut request = client
            .post(format!("{}/{endpoint}", base_url.trim_end_matches('/')))
            .bearer_auth(api_key);
        for (key, value) in headers {
            request = request.header(key, value);
        }
        if let Some(thread_id) = grok_conversation_id.filter(|id| !id.is_empty()) {
            request = request.header("x-grok-conv-id", thread_id);
        }
        let response = tokio::time::timeout(
            idle_timeout,
            request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(payload.bytes.clone())
                .send(),
        )
        .await;
        match response {
            Ok(Ok(response)) if response.status().is_success() => {
                retry_events.push(payload.metadata.clone());
                return Ok(RetriedResponse {
                    response,
                    retry_events,
                    idle_timeout,
                });
            }
            Ok(Ok(response)) => {
                let status = response.status();
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(retry_after_deadline);
                let request_id = crate::stream_diagnostics::provider_request_id(response.headers());
                let text = response.text().await.unwrap_or_default();
                let mut failure = http_failure(status, &text, retry_after);
                failure.request_id = request_id;
                let retryable = failure.kind.is_retryable()
                    && policy
                        .provider_retry_status_codes
                        .contains(&status.as_u16());
                last_error = Some(failure);
                if retryable && attempt < attempts {
                    push_retry_event(
                        &mut retry_events,
                        attempt,
                        &provider_retry_status_cause(status.as_u16()),
                        &policy,
                    );
                    let local_deadline = tokio::time::Instant::now()
                        + Duration::from_millis(provider_retry_delay_ms(&policy, attempt));
                    tokio::time::sleep_until(
                        retry_after.map_or(local_deadline, |server| server.max(local_deadline)),
                    )
                    .await;
                    continue;
                }
            }
            Ok(Err(err)) => {
                let timed_out = err.is_timeout();
                last_error = Some(ProviderFailure::new(
                    ProviderFailureKind::Transport,
                    if timed_out {
                        format!(
                            "OpenAI Responses request timed out after {}ms: {err}",
                            idle_timeout.as_millis()
                        )
                    } else {
                        err.to_string()
                    },
                ));
                if attempt < attempts {
                    let cause = if timed_out {
                        "transport_timeout"
                    } else {
                        "transport_error"
                    };
                    push_retry_event(&mut retry_events, attempt, cause, &policy);
                    retry_sleep(&policy, attempt).await;
                    continue;
                }
            }
            Err(_) => {
                last_error = Some(ProviderFailure::new(
                    ProviderFailureKind::Transport,
                    format!(
                        "OpenAI Responses request timed out waiting for response headers after {}ms",
                        idle_timeout.as_millis()
                    ),
                ));
                if attempt < attempts {
                    push_retry_event(
                        &mut retry_events,
                        attempt,
                        "response_headers_timeout",
                        &policy,
                    );
                    retry_sleep(&policy, attempt).await;
                    continue;
                }
            }
        }
        break;
    }
    let mut failure = last_error.unwrap_or_else(|| {
        ProviderFailure::new(
            ProviderFailureKind::Transport,
            "OpenAI Responses request failed",
        )
    });
    failure.retry_budget_exhausted = true;
    Err(failure.into())
}

pub(super) fn responses_stream_client(idle_timeout: Duration) -> anyhow::Result<reqwest::Client> {
    static CLIENTS: std::sync::OnceLock<std::sync::Mutex<HashMap<Duration, reqwest::Client>>> =
        std::sync::OnceLock::new();
    let mut clients = CLIENTS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| anyhow::anyhow!("Responses client pool lock poisoned"))?;
    if let Some(client) = clients.get(&idle_timeout) {
        return Ok(client.clone());
    }
    let client = reqwest::Client::builder()
        .read_timeout(idle_timeout)
        .build()?;
    // Keep the cache bounded if an embedding changes timeout settings repeatedly.
    if clients.len() >= 8 {
        clients.clear();
    }
    clients.insert(idle_timeout, client.clone());
    Ok(client)
}

/// Capture server advice on receipt; body reads and later callers cannot restart it.
pub(super) fn retry_after_deadline(value: &str) -> Option<tokio::time::Instant> {
    let wall_time = SystemTime::now();
    let received_at = tokio::time::Instant::now();
    let value = value.trim();
    let delay = if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        Duration::from_secs(value.parse().ok()?)
    } else {
        httpdate::parse_http_date(value)
            .ok()?
            .duration_since(wall_time)
            .unwrap_or_default()
    };
    received_at.checked_add(delay)
}

pub(super) fn http_failure(
    status: reqwest::StatusCode,
    body: &str,
    retry_not_before: Option<tokio::time::Instant>,
) -> ProviderFailure {
    let data: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = data.pointer("/error/code").and_then(Value::as_str);
    let kind = failure_kind(code, Some(status.as_u16()));
    ProviderFailure {
        kind,
        message: format!("OpenAI Responses error {status}: {body}"),
        provider_code: code.map(String::from),
        status: Some(status.as_u16()),
        request_id: None,
        response_id: data
            .get("response_id")
            .and_then(Value::as_str)
            .map(String::from),
        retry_not_before,
        retry_budget_exhausted: false,
    }
}

pub(super) fn failure_kind(code: Option<&str>, status: Option<u16>) -> ProviderFailureKind {
    match code {
        Some("context_length_exceeded" | "context_window_exceeded") => {
            ProviderFailureKind::ContextWindowExceeded
        }
        Some("insufficient_quota" | "quota_exceeded") => ProviderFailureKind::QuotaExceeded,
        Some("usage_limit_reached" | "usage_limit_exceeded") => ProviderFailureKind::UsageLimit,
        Some("rate_limit_exceeded" | "rate_limit" | "slow_down") => {
            ProviderFailureKind::RateLimited
        }
        Some("server_overloaded" | "server_error") => ProviderFailureKind::ServerOverloaded,
        Some(
            "flex_unavailable"
            | "invalid_prompt"
            | "invalid_request_error"
            | "cyber_safety"
            | "bio_safety"
            | "misalignment",
        ) => ProviderFailureKind::InvalidRequest,
        _ => match status {
            Some(401 | 403) => ProviderFailureKind::Authentication,
            Some(429) => ProviderFailureKind::RateLimited,
            Some(500..=599) => ProviderFailureKind::ServerOverloaded,
            _ => ProviderFailureKind::InvalidRequest,
        },
    }
}

pub(super) fn sse_failure(data: &Value, kind: &str) -> ProviderFailure {
    let error = data
        .get("error")
        .or_else(|| data.pointer("/response/error"))
        .unwrap_or(data);
    let code = error.get("code").and_then(Value::as_str);
    let mut failure =
        ProviderFailure::new(failure_kind(code, None), stream_error_message(data, kind));
    failure.provider_code = code.map(String::from);
    failure.response_id = data
        .pointer("/response/id")
        .or_else(|| data.get("response_id"))
        .and_then(Value::as_str)
        .map(String::from);
    failure.retry_not_before = error
        .get("retry_after")
        .or_else(|| data.get("retry_after"))
        .and_then(|value| {
            let text = value
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| value.to_string());
            retry_after_deadline(&text)
        });
    failure
}
