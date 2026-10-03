//! One-hop forwarding to a tenant runtime's durable owner.
use std::{fmt, net::SocketAddr, time::Duration};

use futures::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};

pub(crate) const OWNER_HOP_HEADER: &str = "x-roder-owner-hop";
pub(crate) const OWNER_TENANT_HEADER: &str = "x-roder-owner-tenant";

pub(crate) fn tenant_digest(tenant: &str) -> String {
    Sha256::digest(tenant.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A tenant factory may return this error when another replica owns the tenant.
///
/// The host MUST obtain the endpoint from its trusted ownership registry and
/// validate it belongs to this deployment. Never derive it from client input.
/// Replicas must share authentication and request policy. The private transport
/// must protect bearer credentials (for example an encrypted service network).
#[derive(Debug)]
pub struct HostedRuntimeRedirect {
    pub tenant_id: String,
    pub endpoint: SocketAddr,
}

impl fmt::Display for HostedRuntimeRedirect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("tenant runtime is owned by another replica")
    }
}
impl std::error::Error for HostedRuntimeRedirect {}

/// No reconnect or replay: a broken owner connection has an unknown outcome.
pub(crate) async fn relay_to_owner(
    mut client: WebSocketStream<TcpStream>,
    endpoint: SocketAddr,
    bearer: &str,
    tenant: &str,
) -> anyhow::Result<()> {
    let mut request = format!("ws://{endpoint}/").into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {bearer}").parse()?);
    request.headers_mut().insert(OWNER_HOP_HEADER, "1".parse()?);
    request
        .headers_mut()
        .insert(OWNER_TENANT_HEADER, tenant_digest(tenant).parse()?);
    request.headers_mut().insert(
        "sec-websocket-protocol",
        crate::remote::REMOTE_PROTOCOL.parse()?,
    );
    // Deliberately discard transport errors: handshake errors may contain the
    // original request, including its bearer. Audit only a fixed reason code.
    let connected = tokio::time::timeout(
        Duration::from_secs(3),
        tokio_tungstenite::connect_async(request),
    )
    .await;
    let Ok(Ok((mut owner, _))) = connected else {
        let _ = client.close(None).await;
        anyhow::bail!("owner_connection_unavailable");
    };
    loop {
        tokio::select! {
            incoming = client.next() => {
                let Some(Ok(message)) = incoming else { break };
                let closing = matches!(message, Message::Close(_));
                if owner.send(message).await.is_err() || closing { break; }
            }
            outgoing = owner.next() => {
                let Some(Ok(message)) = outgoing else { break };
                let closing = matches!(message, Message::Close(_));
                if client.send(message).await.is_err() || closing { break; }
            }
        }
    }
    // The socket drops also tear down the peer. Do not wait for a second close
    // handshake, which could hold a draining replica indefinitely.
    Ok(())
}
