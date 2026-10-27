use std::net::TcpStream;
use std::time::Duration;

use tokio_tungstenite::tungstenite::stream::MaybeTlsStream;
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message, WebSocket};

use crate::error::DaemonError;

pub type CodexSocket = WebSocket<MaybeTlsStream<TcpStream>>;

pub(super) fn set_socket_timeouts(
    socket: &mut CodexSocket,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
) -> Result<(), DaemonError> {
    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream
            .set_read_timeout(read_timeout)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "codex_socket_timeout",
                message: error.to_string(),
            })?;
        stream
            .set_write_timeout(write_timeout)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "codex_socket_timeout",
                message: error.to_string(),
            })?;
    }
    Ok(())
}

// Tungstenite owns this error representation, so boxing it here would only push an allocation
// onto every nonblocking read while callers still need the concrete variants for control flow.
#[allow(clippy::result_large_err)]
pub(super) fn read_socket_nonblocking(socket: &mut CodexSocket) -> Result<Message, WebSocketError> {
    let MaybeTlsStream::Plain(stream) = socket.get_mut() else {
        return socket.read();
    };
    stream.set_nonblocking(true)?;
    let result = socket.read();
    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream.set_nonblocking(false)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};

    use tokio_tungstenite::tungstenite::{accept, connect, Error as WebSocketError};

    use super::read_socket_nonblocking;

    #[test]
    fn nonblocking_read_returns_without_waiting_for_a_websocket_message() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind websocket listener");
        let address = listener.local_addr().expect("read websocket address");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept websocket client");
            let _socket = accept(stream).expect("accept websocket handshake");
            thread::sleep(Duration::from_millis(250));
        });
        let (mut socket, _) = connect(format!("ws://{address}")).expect("connect websocket client");

        let started_at = Instant::now();
        let result = read_socket_nonblocking(&mut socket);

        assert!(matches!(
            result,
            Err(WebSocketError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        assert!(started_at.elapsed() < Duration::from_millis(100));
        server.join().expect("join websocket server");
    }
}
