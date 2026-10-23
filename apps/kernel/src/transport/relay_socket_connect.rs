//! MP-08 / MP-10 / MP-11: bound the stack used to construct relay TLS futures.
use std::{future::Future, pin::Pin};

use crate::outbound_dial::{connect_websocket, WebSocket};
use tokio_tungstenite::tungstenite::{handshake::client::Response, Error};

type ConnectFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(WebSocket, Response), Error>> + Send + 'a>>;

// Keep construction out of callers' poll frames, including their timeout and
// cancellation wrappers. The normal relay connection remains deferred.
#[inline(never)]
pub(super) fn connect(relay_url: &str) -> ConnectFuture<'_> {
    Box::pin(connect_websocket(
        chariox_relay::frame_transport::transport_url(relay_url),
    ))
}

// MP-08 / MP-10 / MP-11: negotiate both display and bounded framing without
// constructing the TLS future in the connector's poll frame.
#[inline(never)]
pub(super) fn connect_binary_events(relay_url: &str) -> ConnectFuture<'_> {
    match chariox_relay::binary_event::connection_request(
        &chariox_relay::frame_transport::transport_url(relay_url),
    ) {
        Ok(request) => Box::pin(connect_websocket(request)),
        Err(error) => Box::pin(async move { Err(error) }),
    }
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
