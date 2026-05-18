use std::net::TcpStream;
use std::time::Duration;

use tokio_tungstenite::tungstenite::stream::MaybeTlsStream;
use tokio_tungstenite::tungstenite::WebSocket;

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
