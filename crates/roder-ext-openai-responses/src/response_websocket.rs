use super::*;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_tungstenite::tungstenite::{
    Message, client::IntoClientRequest, protocol::WebSocketConfig,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

// Credentials participate in connection identity, but this type intentionally
// has no Debug implementation and is never emitted in telemetry.
#[derive(Clone, PartialEq, Eq, Hash)]
struct SessionKey {
    base_url: String,
    thread_id: String,
    token: String,
    headers: Vec<(String, String)>,
}

#[derive(Default)]
struct Session {
    socket: Option<Socket>,
    last: Option<LastResponse>,
    http_until: Option<tokio::time::Instant>,
    routing_hint: Option<String>,
    request_id: Option<String>,
}

struct LastResponse {
    request: Value,
    output: Vec<Value>,
    id: String,
}

pub(super) fn websocket_requested(base_url: &str) -> bool {
    match std::env::var("RODER_RESPONSES_TRANSPORT").as_deref() {
        Ok("http") => false,
        Ok("websocket") => true,
        _ => reqwest::Url::parse(base_url)
            .ok()
            .is_some_and(|url| matches!(url.host_str(), Some("api.openai.com" | "chatgpt.com"))),
    }
}

fn session(key: SessionKey) -> anyhow::Result<Arc<Mutex<Session>>> {
    static SESSIONS: std::sync::OnceLock<
        std::sync::Mutex<HashMap<SessionKey, Arc<Mutex<Session>>>>,
    > = std::sync::OnceLock::new();
    let mut sessions = SESSIONS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| anyhow::anyhow!("Responses WebSocket session pool lock poisoned"))?;
    if let Some(session) = sessions.get(&key) {
        return Ok(session.clone());
    }
    if sessions.len() >= 64 {
        // Eviction only closes idle sessions; active streams own another Arc.
        sessions.retain(|_, session| Arc::strong_count(session) > 1);
    }
    let session = Arc::new(Mutex::new(Session::default()));
    sessions.insert(key, session.clone());
    Ok(session)
}

pub(super) async fn try_websocket_stream(
    base_url: &str,
    token: &str,
    headers: &[(String, String)],
    thread_id: &str,
    body: &Value,
    tool_name_map: HashMap<String, String>,
    continuation: Option<ClientToolSearchContext>,
) -> anyhow::Result<Option<InferenceEventStream>> {
    let session = session(SessionKey {
        base_url: base_url.into(),
        thread_id: thread_id.into(),
        token: token.into(),
        headers: headers.to_vec(),
    })?;
    let mut lease = session.lock_owned().await;
    if lease
        .http_until
        .is_some_and(|until| until > tokio::time::Instant::now())
    {
        return Ok(None);
    }
    let reused = lease.socket.is_some();
    let socket = if let Some(socket) = lease.socket.take() {
        socket
    } else {
        let mut url =
            reqwest::Url::parse(&format!("{}/responses", base_url.trim_end_matches('/')))?;
        url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
            .map_err(|_| anyhow::anyhow!("invalid Responses WebSocket scheme"))?;
        let mut request = url.as_str().into_client_request()?;
        let h = request.headers_mut();
        h.insert("authorization", format!("Bearer {token}").parse()?);
        h.insert("openai-beta", "responses_websockets=2026-02-06".parse()?);
        h.insert("session_id", thread_id.parse()?);
        for (name, value) in headers {
            h.insert(
                name.parse::<tokio_tungstenite::tungstenite::http::HeaderName>()?,
                value.parse()?,
            );
        }
        if let Some(hint) = &lease.routing_hint {
            h.insert("x-codex-turn-state", hint.parse()?);
        }
        let config = WebSocketConfig::default()
            .max_message_size(Some(16 * 1024 * 1024))
            .max_frame_size(Some(16 * 1024 * 1024));
        match tokio::time::timeout(
            responses_stream_idle_timeout(),
            tokio_tungstenite::connect_async_with_config(request, Some(config), true),
        )
        .await
        {
            Ok(Ok((socket, response))) => {
                lease.request_id =
                    crate::stream_diagnostics::provider_request_id(response.headers());
                lease.routing_hint = response
                    .headers()
                    .get("x-codex-turn-state")
                    .and_then(|value| value.to_str().ok())
                    .map(String::from);
                socket
            }
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) => {
                let status = reqwest::StatusCode::from_u16(response.status().as_u16())?;
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .and_then(retry_after_deadline);
                if matches!(status.as_u16(), 404 | 405 | 426 | 501) {
                    lease.last = None;
                    lease.http_until = Some(tokio::time::Instant::now() + Duration::from_secs(300));
                    if let Some(deadline) = retry_after {
                        tokio::time::sleep_until(deadline).await;
                    }
                    return Ok(None);
                }
                let body = response
                    .body()
                    .as_deref()
                    .map(String::from_utf8_lossy)
                    .unwrap_or_default();
                let mut failure = http_failure(status, &body, retry_after);
                failure.request_id =
                    crate::stream_diagnostics::provider_request_id(response.headers());
                return Err(failure.into());
            }
            Ok(Err(_)) | Err(_) => {
                lease.last = None;
                lease.http_until = Some(tokio::time::Instant::now() + Duration::from_secs(300));
                return Ok(None);
            }
        }
    };
    let body = body.clone();
    let last = lease.last.take();
    Ok(Some(websocket_stream(
        socket,
        lease,
        body,
        last,
        tool_name_map,
        reused,
        continuation,
    )))
}

fn incremental_request(body: &Value, last: Option<&LastResponse>) -> Value {
    let mut request = body.clone();
    if let Some(last) = last {
        let mut properties = body.clone();
        let mut prior_properties = last.request.clone();
        properties
            .as_object_mut()
            .map(|fields| fields.remove("input"));
        prior_properties
            .as_object_mut()
            .map(|fields| fields.remove("input"));
        if properties == prior_properties {
            let prior = last.request.get("input").and_then(Value::as_array);
            let next = body.get("input").and_then(Value::as_array);
            if let (Some(prior), Some(next)) = (prior, next) {
                let expected: Vec<_> = prior.iter().chain(&last.output).collect();
                if next.len() >= expected.len()
                    && expected.iter().zip(next).all(|(old, new)| *old == new)
                {
                    request["input"] = json!(&next[expected.len()..]);
                    request["previous_response_id"] = json!(last.id);
                }
            }
        }
    }
    request["type"] = json!("response.create");
    request
}

fn websocket_stream(
    mut socket: Socket,
    mut lease: OwnedMutexGuard<Session>,
    mut body: Value,
    mut last: Option<LastResponse>,
    tool_name_map: HashMap<String, String>,
    reused: bool,
    mut continuation: Option<ClientToolSearchContext>,
) -> InferenceEventStream {
    Box::pin(async_stream::try_stream! {
        let mut search_history = Vec::new();
        let mut state = ResponsesStreamState {tool_name_map,..Default::default()};
        'requests: for round in 0..=MAX_CLIENT_TOOL_SEARCH_ROUNDS {
            let mut request = incremental_request(&body,last.as_ref());
            let mut repaired_previous = false;
            loop {
                let payload = prepare_request_payload(&request,request_byte_limit())?;
                let cacheable = payload.metadata["shed_tool_images"] == 0;
                socket.send(Message::Text(std::str::from_utf8(&payload.bytes)?.to_string().into())).await.map_err(|error| socket_failure(&mut lease,error.to_string()))?;
                yield InferenceEvent::ProviderMetadata(payload.metadata);
                yield InferenceEvent::ProviderMetadata(json!({"type":"responses_transport","transport":"websocket","connection_reused":reused || round > 0,"incremental":request.get("previous_response_id").is_some()}));
                state.terminal = false;
                state.response_id = None;
                state.successful = false;
                state.streamed_final_text = false;
                state.current_message_phase.clear();
                state.response_output.clear();
                let mut emitted_output = false;
                loop {
                    let data = websocket_event(&mut socket,&mut lease).await.map_err(|mut error| {
                        if let Some(failure) = error.downcast_mut::<ProviderFailure>() {failure.response_id = state.response_id.clone();}
                        error
                    })?;
                    let code = data.pointer("/error/code").or_else(||data.pointer("/response/error/code")).and_then(Value::as_str);
                    if code == Some("previous_response_not_found") && request.get("previous_response_id").is_some() && !repaired_previous && !emitted_output {
                        repaired_previous = true;
                        request = incremental_request(&body,None);
                        break;
                    }
                    let event = SseEvent {event:None,data};
                    let mut events = events_from_sse_event(&event,&mut state);
                    if let Some(message) = state.protocol_failure.take() { Err(ProviderFailure::new(ProviderFailureKind::Protocol,message))?; }
                    if state.successful {
                        let response = event.data.get("response").unwrap_or(&event.data);
                        let next_last = response.get("id").and_then(Value::as_str).filter(|_|cacheable)
                            .map(|id|LastResponse {request:body.clone(),output:state.response_output.clone(),id:id.into()});
                        let pending = std::mem::take(&mut state.pending_client_tool_searches);
                        if !pending.is_empty() && let Some(ctx) = continuation.as_mut() {
                            for inference in events {
                                if matches!(inference,InferenceEvent::Completed(_)) || matches!(&inference,InferenceEvent::ProviderMetadata(metadata) if metadata.get("output").is_some()) { continue; }
                                yield inference;
                            }
                            if round == MAX_CLIENT_TOOL_SEARCH_ROUNDS {
                                Err(ProviderFailure::new(ProviderFailureKind::ToolSearchExhausted,"Responses client tool-search continuation limit exhausted"))?;
                            }
                            let execution = execute_client_searches(ctx,&mut state,pending)?;
                            search_history.extend(execution.history);
                            for inference in execution.events { yield inference; }
                            body = ctx.body.clone();
                            last = next_last;
                            continue 'requests;
                        }
                        for inference in &mut events {
                            if let InferenceEvent::ProviderMetadata(metadata) = inference && metadata.get("output").is_some() {
                                let mut output = search_history.clone();
                                output.extend(state.response_output.clone());
                                metadata["output"] = json!(output);
                            }
                        }
                        // Cache only a terminal response; interrupted streams own
                        // and drop their socket rather than leaving it reusable.
                        lease.last = next_last;
                        lease.socket = Some(socket);
                        for inference in events { yield inference; }
                        return;
                    }
                    for inference in events {
                        if matches!(inference,InferenceEvent::Failed(_)) {
                            let mut failure = sse_failure(&event.data,"response.failed");
                            failure.request_id = lease.request_id.clone();
                            if failure.response_id.is_none() {failure.response_id=state.response_id.clone();}
                            Err(failure)?;
                        }
                        emitted_output = true;
                        yield inference;
                    }
                }
            }
        }
    })
}

async fn websocket_event(socket: &mut Socket, lease: &mut Session) -> anyhow::Result<Value> {
    loop {
        let message = tokio::time::timeout(responses_stream_idle_timeout(), socket.next())
            .await
            .map_err(|_| socket_failure(lease, "Responses WebSocket idle timeout".into()))?
            .ok_or_else(|| {
                socket_failure(
                    lease,
                    "Responses WebSocket closed before response.completed".into(),
                )
            })?
            .map_err(|error| socket_failure(lease, error.to_string()))?;
        let data = match message {
            Message::Text(text) => text.to_string(),
            Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).map_err(|error| {
                ProviderFailure::new(ProviderFailureKind::Protocol, error.to_string())
            })?,
            Message::Ping(bytes) => {
                socket
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|error| socket_failure(lease, error.to_string()))?;
                continue;
            }
            Message::Pong(_) | Message::Frame(_) => continue,
            Message::Close(_) => {
                return Err(socket_failure(
                    lease,
                    "Responses WebSocket closed before response.completed".into(),
                )
                .into());
            }
        };
        return serde_json::from_str(&data).map_err(|error| {
            ProviderFailure::new(
                ProviderFailureKind::Protocol,
                format!("invalid Responses WebSocket event: {error}"),
            )
            .into()
        });
    }
}

fn socket_failure(lease: &mut Session, message: String) -> ProviderFailure {
    lease.last = None;
    lease.http_until = Some(tokio::time::Instant::now() + Duration::from_secs(300));
    let mut failure = ProviderFailure::new(ProviderFailureKind::StreamInterrupted, message);
    failure.request_id = lease.request_id.clone();
    failure
}

#[cfg(test)]
#[path = "websocket_tests.rs"]
mod tests;
