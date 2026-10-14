//! Loopback upgrade policy and the first-message grant/key handshake.

use std::collections::VecDeque;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::WebSocketStream;

use super::*;

const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(5);
const MAX_HANDSHAKE_FRAME_BYTES: usize = 16 * 1024;
const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MAX_PENDING_HANDSHAKES: usize = 8;
const MAX_SESSIONS: usize = 32;
const FORWARDING_HEADERS: [&str; 5] = [
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-real-ip",
];

pub(super) type LocalBrowserSocket = WebSocketStream<TcpStream>;

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum ServerHandshakeFrame<'a> {
    LocalChallenge {
        kernel_id: &'a str,
        endpoint_epoch: &'a str,
        challenge: &'a str,
    },
    LocalConnected {
        daemon_public_key: String,
        proof: EncryptedRelayPayload,
    },
    Close {
        reason: &'a str,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ClientHandshakeFrame {
    LocalConnect { proof: EncryptedRelayPayload },
}

/// Encrypted from the identity-bound browser key to the kernel key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientProof {
    grant: String,
    challenge: String,
    origin: String,
    kernel_id: String,
    endpoint_epoch: String,
}

/// Encrypted from the kernel key to the browser key: only the holder of the
/// pinned kernel key can learn the grant and answer with it.
#[derive(Debug, Serialize)]
struct KernelProof<'a> {
    grant: &'a str,
    challenge: &'a str,
}

pub(super) async fn run_when_bound(
    direct: Arc<LocalBrowserDirect>,
    mut bound: mpsc::UnboundedReceiver<(TcpListener, LocalBrowserEndpoint)>,
) {
    let mut shutdown = direct.shutdown.clone();
    let (listener, endpoint) = tokio::select! {
        _ = shutdown.changed() => return,
        bound = bound.recv() => match bound {
            Some(bound) => bound,
            None => return,
        },
    };
    tokio::join!(
        run_listener(Arc::clone(&direct), listener, endpoint),
        direct.watch_authority(),
    );
}

async fn run_listener(
    direct: Arc<LocalBrowserDirect>,
    listener: TcpListener,
    endpoint: LocalBrowserEndpoint,
) {
    let mut handshakes = VecDeque::<tokio::task::AbortHandle>::new();
    let sessions = Arc::new(Semaphore::new(MAX_SESSIONS));
    let health = direct.router.transport_health_store();
    let mut shutdown = direct.shutdown.clone();
    loop {
        let (stream, peer) = tokio::select! {
            _ = shutdown.changed() => return,
            accepted = crate::transport::listener_admission::accept_with_backoff(
                &listener, &health, "local browser websocket",
            ) => accepted,
        };
        if *shutdown.borrow() {
            return;
        }
        let Ok(session) = Arc::clone(&sessions).try_acquire_owned() else {
            refuse(peer, "local browser endpoint is at capacity");
            continue;
        };
        // Idle local connections cannot pin the endpoint: at capacity the
        // oldest unauthenticated handshake makes room for the newest.
        handshakes.retain(|handshake| !handshake.is_finished());
        if handshakes.len() >= MAX_PENDING_HANDSHAKES {
            if let Some(oldest) = handshakes.pop_front() {
                oldest.abort();
            }
        }
        let direct = Arc::clone(&direct);
        let endpoint = endpoint.clone();
        let handshake = tokio::spawn(async move {
            let admitted = timeout(HANDSHAKE_DEADLINE, admit(&direct, &endpoint, stream, peer))
                .await
                .unwrap_or(Err("local browser handshake timed out"));
            match admitted {
                Ok((socket, grant)) => {
                    tokio::spawn(async move {
                        session::run(direct, socket, grant).await;
                        drop(session);
                    });
                }
                Err(reason) => refuse(peer, reason),
            }
        });
        handshakes.push_back(handshake.abort_handle());
    }
}

async fn admit(
    direct: &LocalBrowserDirect,
    endpoint: &LocalBrowserEndpoint,
    stream: TcpStream,
    peer: SocketAddr,
) -> Result<(LocalBrowserSocket, LocalBrowserGrant), &'static str> {
    if !peer.ip().is_loopback() {
        return Err("local browser peer is not loopback");
    }
    let origin = direct
        .authority
        .borrow()
        .as_ref()
        .map(|authority| authority.origin.clone())
        .ok_or("kernel is not paired with a Cloud origin")?;
    let expected_host = format!("127.0.0.1:{}", endpoint.port);
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_MESSAGE_BYTES);
    config.max_frame_size = Some(MAX_MESSAGE_BYTES);
    let mut upgrade_refusal = None;
    let socket = tokio_tungstenite::accept_hdr_async_with_config(
        stream,
        |request: &Request, response: Response| match upgrade_policy(
            request,
            &expected_host,
            &origin,
        ) {
            Ok(()) => Ok(response),
            Err(reason) => {
                upgrade_refusal = Some(reason);
                let mut error = ErrorResponse::new(Some("Forbidden".to_string()));
                *error.status_mut() = StatusCode::FORBIDDEN;
                Err(error)
            }
        },
        Some(config),
    )
    .await;
    let mut socket = match socket {
        Ok(socket) => socket,
        Err(_) => return Err(upgrade_refusal.unwrap_or("local browser upgrade failed")),
    };
    let kernel_id = direct.router.relay_daemon_id();
    let challenge = random_token();
    send_frame(
        &mut socket,
        &ServerHandshakeFrame::LocalChallenge {
            kernel_id: &kernel_id,
            endpoint_epoch: &endpoint.epoch,
            challenge: &challenge,
        },
    )
    .await?;
    let result = verify_connect(
        direct,
        endpoint,
        &mut socket,
        &kernel_id,
        &challenge,
        &origin,
    )
    .await;
    match result {
        Ok((grant, proof)) => {
            send_frame(
                &mut socket,
                &ServerHandshakeFrame::LocalConnected {
                    daemon_public_key: direct.router.relay_config_snapshot().relay_public_key,
                    proof,
                },
            )
            .await?;
            Ok((socket, grant))
        }
        Err(reason) => {
            let _ = send_frame(&mut socket, &ServerHandshakeFrame::Close { reason }).await;
            let _ = socket.close(None).await;
            Err(reason)
        }
    }
}

/// Exact literal loopback Host and path, exactly one paired Origin, and no
/// proxy headers. `null`, absent, duplicate or look-alike Origins are refused.
fn upgrade_policy(
    request: &Request,
    expected_host: &str,
    origin: &str,
) -> Result<(), &'static str> {
    if request.uri().path() != LOCAL_BROWSER_PATH || request.uri().query().is_some() {
        return Err("local browser path is not admitted");
    }
    let headers = request.headers();
    let mut hosts = headers.get_all("host").iter();
    if hosts.next().and_then(|host| host.to_str().ok()) != Some(expected_host)
        || hosts.next().is_some()
    {
        return Err("local browser host is not the literal loopback endpoint");
    }
    let mut origins = headers.get_all("origin").iter();
    if origins.next().and_then(|value| value.to_str().ok()) != Some(origin)
        || origins.next().is_some()
    {
        return Err("local browser origin is not the paired Cloud origin");
    }
    if FORWARDING_HEADERS
        .iter()
        .any(|name| headers.contains_key(*name))
    {
        return Err("local browser upgrade carried proxy headers");
    }
    Ok(())
}

async fn verify_connect(
    direct: &LocalBrowserDirect,
    endpoint: &LocalBrowserEndpoint,
    socket: &mut LocalBrowserSocket,
    kernel_id: &str,
    challenge: &str,
    origin: &str,
) -> Result<(LocalBrowserGrant, EncryptedRelayPayload), &'static str> {
    let payload = loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => break text,
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return Err("local browser closed before authenticating"),
        }
    };
    if payload.len() > MAX_HANDSHAKE_FRAME_BYTES {
        return Err("local browser handshake frame is too large");
    }
    let ClientHandshakeFrame::LocalConnect { proof } =
        serde_json::from_str(&payload).map_err(|_| "local browser handshake frame is invalid")?;
    let decrypted =
        relay_crypto::decrypt_payload_for_private_key(&direct.router.relay_private_key(), &proof)
            .map_err(|_| "local browser proof is not for this kernel key")?;
    let claimed: ClientProof = serde_json::from_slice(&decrypted.plaintext)
        .map_err(|_| "local browser proof is invalid")?;
    if claimed.challenge != challenge
        || claimed.kernel_id != kernel_id
        || claimed.endpoint_epoch != endpoint.epoch
        || claimed.origin != origin
    {
        return Err("local browser proof is not bound to this connection");
    }
    let thumbprint =
        crate::runtime::terminal_pairings::public_key_thumbprint(&decrypted.sender_public_key);
    let grant = direct.redeem_grant(&claimed.grant, &thumbprint)?;
    if grant.authority.origin != origin {
        return Err("local browser grant was issued for another origin");
    }
    let kernel_proof = serde_json::to_vec(&KernelProof {
        grant: &claimed.grant,
        challenge,
    })
    .map_err(|_| "local browser proof could not be encoded")?;
    let proof = relay_crypto::encrypt_payload_for_peer(
        &direct.router.relay_private_key(),
        &decrypted.sender_public_key,
        &kernel_proof,
    )
    .map_err(|_| "local browser proof could not be encrypted")?;
    Ok((grant, proof))
}

async fn send_frame(
    socket: &mut LocalBrowserSocket,
    frame: &ServerHandshakeFrame<'_>,
) -> Result<(), &'static str> {
    let text =
        serde_json::to_string(frame).map_err(|_| "local browser frame could not be encoded")?;
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| "local browser closed during handshake")
}

fn refuse(peer: SocketAddr, reason: &'static str) {
    crate::logging::warn_with_fields(
        "daemon.local_browser",
        "local browser connection refused",
        serde_json::json!({"peer": peer.to_string(), "reason": reason}),
    );
}
