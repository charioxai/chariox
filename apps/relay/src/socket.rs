//! MP-08 / MP-10 / MP-11: shared relay connection transport policy.
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{handshake::client::Response, Error},
    MaybeTlsStream, WebSocketStream,
};

pub async fn connect_relay_socket(
    url: &str,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error> {
    // Interactive request/response tails must not wait for a delayed TCP ACK.
    connect_async_with_config(&crate::frame_transport::transport_url(url), None, true).await
}

// MP-08 / MP-10 / MP-11: one transport policy for every relay role.
pub async fn accept_relay_socket(stream: TcpStream) -> Result<WebSocketStream<TcpStream>, Error> {
    stream.set_nodelay(true)?;
    tokio_tungstenite::accept_async(stream).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn relay_socket_sends_short_frames_without_nagle() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            accept_relay_socket(stream).await.unwrap()
        });
        let (socket, _) = connect_relay_socket(&url).await.unwrap();
        let MaybeTlsStream::Plain(tcp) = socket.get_ref() else {
            panic!("expected plain test socket")
        };
        let accepted = server.await.unwrap();
        let client_no_delay = tcp.nodelay().unwrap();
        let server_no_delay = accepted.get_ref().nodelay().unwrap();
        assert_eq!(
            (client_no_delay, server_no_delay),
            (true, true),
            "both relay endpoints must send short frames without waiting for a delayed ACK"
        );
    }
}
