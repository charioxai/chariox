//! MP-08 / MP-10 / MP-11: bound the stack used to construct relay TLS futures.
use std::{future::Future, pin::Pin};

use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{handshake::client::Response, Error},
    MaybeTlsStream, WebSocketStream,
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type ConnectFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(Socket, Response), Error>> + Send + 'a>>;

// Keep construction out of callers' poll frames, including their timeout and
// cancellation wrappers. The normal relay connection remains deferred.
#[inline(never)]
pub(super) fn connect(relay_url: &str) -> ConnectFuture<'_> {
    Box::pin(connect_async_with_config(
        chariox_relay::frame_transport::transport_url(relay_url),
        None,
        true,
    ))
}

// MP-08 / MP-10 / MP-11: negotiate both display and bounded framing without
// constructing the TLS future in the connector's poll frame.
#[inline(never)]
pub(super) fn connect_binary_events(relay_url: &str) -> ConnectFuture<'_> {
    match chariox_relay::binary_event::connection_request(
        &chariox_relay::frame_transport::transport_url(relay_url),
    ) {
        Ok(request) => Box::pin(connect_async_with_config(request, None, true)),
        Err(error) => Box::pin(async move { Err(error) }),
    }
}

#[cfg(test)]
mod tests {
    // MP-08/MP-10/MP-11: Room desktop works without the user-domain browser
    // flag. Its ordinary relay socket must not add Nagle delay to input or frames.
    #[tokio::test]
    async fn room_relay_socket_sends_small_packets_without_nagle_delay() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(stream).await.unwrap()
        });
        let (socket, _) = super::connect(&format!("ws://{address}")).await.unwrap();
        let tokio_tungstenite::MaybeTlsStream::Plain(stream) = socket.get_ref() else {
            panic!("local socket must be plain TCP");
        };
        assert!(
            stream.nodelay().unwrap(),
            "Room packets must leave immediately"
        );
        drop(socket);
        drop(server.await.unwrap());
    }
}
