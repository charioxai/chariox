//! MP-08 / MP-10 / MP-11: bound the stack used to construct relay TLS futures.
use std::{future::Future, pin::Pin};

use chariox_relay::protocol::{DaemonRegistration, RelayEnvelope};
use chariox_relay::registration_proof;

use crate::error::DaemonError;
use crate::outbound_dial::{connect_websocket, WebSocket};
use crate::transport::relay_crypto;
use tokio_tungstenite::tungstenite::{
    handshake::client::{Request, Response},
    Error,
};

type ConnectFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(WebSocket, Response), Error>> + Send + 'a>>;

// Keep construction out of callers' poll frames, including their timeout and
// cancellation wrappers. The normal relay connection remains deferred.
#[inline(never)]
pub(crate) fn connect(relay_url: &str) -> ConnectFuture<'_> {
    Box::pin(connect_websocket(
        chariox_relay::frame_transport::transport_url(relay_url),
    ))
}

// MP-08 / MP-10 / MP-11: negotiate binary frames, bounded framing and chunked
// uploads without constructing the TLS future in the caller's poll frame. The
// kernel's own relay connection and the gateway's client links both use it.
#[inline(never)]
pub(crate) fn connect_binary_events(relay_url: &str) -> ConnectFuture<'_> {
    match chariox_relay::binary_event::connection_request(
        &chariox_relay::frame_transport::transport_url(relay_url),
    ) {
        Ok(request) => Box::pin(connect_websocket(offer_upload_chunks(
            chariox_relay::binary_request::offer(request),
        ))),
        Err(error) => Box::pin(async move { Err(error) }),
    }
}

fn offer_upload_chunks(mut request: Request) -> Request {
    request.headers_mut().insert(
        chariox_relay::frame_transport::UPLOAD_HEADER,
        chariox_relay::frame_transport::UPLOAD_CHUNKS
            .parse()
            .expect("a valid header value"),
    );
    request
}

/// True when the relay answered the binary frame offer in its handshake.
pub(crate) fn binary_events_accepted(handshake: &Response) -> bool {
    handshake
        .headers()
        .get(chariox_relay::binary_event::VERSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok())
        .is_some_and(|version| version >= chariox_relay::binary_event::PROTOCOL_VERSION)
}

/// True when the relay answered the binary request offer: requests then travel as `CXR3`
/// frames in both directions of this connection.
pub(crate) fn binary_requests_accepted(handshake: &Response) -> bool {
    handshake
        .headers()
        .get(chariox_relay::binary_request::HEADER)
        .is_some_and(|value| value == chariox_relay::binary_request::OFFER)
}

/// True when the relay answered the upload offer in its handshake.
pub(crate) fn upload_chunks_accepted(handshake: &Response) -> bool {
    handshake
        .headers()
        .get(chariox_relay::frame_transport::UPLOAD_HEADER)
        .is_some_and(|value| value == chariox_relay::frame_transport::UPLOAD_CHUNKS)
}

/// The registration this connection admits: signed with the kernel identity key for the
/// challenge the relay offered in `handshake`.
pub(crate) fn daemon_register(
    relay_url: &str,
    handshake: &Response,
    registration: DaemonRegistration,
    private_key_base64: &str,
) -> Result<RelayEnvelope, DaemonError> {
    let refused = |message: String| DaemonError::LocalTransport {
        operation: "prove relay registration",
        message,
    };
    let challenge = registration_proof::offered_challenge(handshake).ok_or_else(|| {
        refused("the relay offered no registration challenge; update the relay".to_string())
    })?;
    let relay_address = registration_proof::dialed_address(relay_url).map_err(refused)?;
    let identity_key = relay_crypto::decode_private_key(private_key_base64)?;
    let proof = registration_proof::prove(&identity_key, &relay_address, &registration, challenge);
    Ok(RelayEnvelope::DaemonRegister {
        registration,
        proof,
    })
}

/// A stand-in relay's handshake: it offers a registration challenge as the relay does.
#[cfg(test)]
// tungstenite's upgrade callback fixes the large `ErrorResponse` error type.
#[allow(clippy::result_large_err)]
pub(crate) async fn accept_as_relay<S>(
    stream: S,
) -> Result<tokio_tungstenite::WebSocketStream<S>, Error>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio_tungstenite::accept_hdr_async(
        stream,
        |_: &tokio_tungstenite::tungstenite::handshake::server::Request,
         mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
            response.headers_mut().insert(
                registration_proof::CHALLENGE_HEADER,
                "stand-in-relay-challenge".parse().unwrap(),
            );
            Ok(response)
        },
    )
    .await
}

/// A stand-in relay accepts the test kernel only after reading its registration.
#[cfg(test)]
pub(crate) async fn acknowledge_test_registration<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let message = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(message, Message::Text(text) if matches!(
        serde_json::from_str(&text),
        Ok(RelayEnvelope::DaemonRegister { .. })
    )));
    socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegistered)
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
}

/// A registration for a test that stands in for a kernel on a real relay: `registration` is the
/// wire JSON without a public key, which a fresh identity key fills in and signs.
#[cfg(test)]
pub(crate) fn stand_in_kernel_register(
    relay_url: &str,
    handshake: &Response,
    mut registration: serde_json::Value,
) -> String {
    let private_key = relay_crypto::generate_private_key_base64();
    registration["public_key"] = relay_crypto::public_key_from_private_key_base64(&private_key)
        .unwrap()
        .into();
    let registration = serde_json::from_value(registration).unwrap();
    serde_json::to_string(
        &daemon_register(relay_url, handshake, registration, &private_key).unwrap(),
    )
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    // MP-08 / MP-10 / MP-11: ordinary pane traffic needs the same immediate TCP delivery.
    #[tokio::test]
    async fn relay_connections_disable_nagle_without_browser_display() {
        crate::test_support::isolated_env_test!();
        std::env::remove_var("CHARIOX_KERNEL_BROWSER_DISPLAY");
        for binary_events in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                tokio_tungstenite::accept_hdr_async(
                    stream,
                    |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                     mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                        if let Some(protocol) = request.headers().get("sec-websocket-protocol") {
                            response.headers_mut().insert("sec-websocket-protocol", protocol.clone());
                        }
                        Ok(response)
                    },
                ).await.unwrap()
            });
            let connecting = if binary_events {
                connect_binary_events(&url)
            } else {
                connect(&url)
            };
            let (socket, _) = tokio::time::timeout(std::time::Duration::from_secs(5), connecting)
                .await
                .unwrap()
                .unwrap();
            let immediate = match socket.get_ref() {
                tokio_tungstenite::MaybeTlsStream::Plain(stream) => stream.nodelay().unwrap(),
                _ => panic!("loopback fixture must use plain TCP"),
            };
            let peer = server.await.unwrap();
            drop(peer);
            drop(socket);
            assert!(immediate, "pane delivery must disable Nagle without a Browser flag (binary events={binary_events})");
        }
    }
}
