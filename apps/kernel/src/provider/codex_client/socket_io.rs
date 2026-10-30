use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message, WebSocket};

use crate::error::DaemonError;

pub type CodexSocket = WebSocket<CodexStream>;

/// The app-server connection, read one websocket frame at a time: the socket
/// never takes bytes past the frame it is reading, so between two messages
/// everything unread is still in the connection itself. A hot reload hands
/// the connection over with only the bytes of a message it is in the middle of
/// (`unread`), and the next kernel program continues reading the same stream.
#[derive(Debug)]
pub struct CodexStream {
    tcp: TcpStream,
    framing: Framing,
    // The bytes the socket took since the last complete message.
    unread: Vec<u8>,
}

#[derive(Debug)]
enum Framing {
    // The HTTP upgrade: read freely, since the handshake keeps what follows it.
    Handshake,
    Header { bytes: [u8; 14], have: usize },
    Payload { remaining: u64, frame: (u8, bool) },
}

impl Framing {
    fn header() -> Self {
        Self::Header {
            bytes: [0; 14],
            have: 0,
        }
    }
}

impl CodexStream {
    pub fn new(tcp: TcpStream) -> Self {
        Self {
            tcp,
            framing: Framing::Handshake,
            unread: Vec::new(),
        }
    }

    /// A connection whose upgrade is done, continued after `unread`.
    pub fn upgraded(tcp: TcpStream, unread: &[u8]) -> Self {
        let mut stream = Self {
            tcp,
            framing: Framing::header(),
            unread: Vec::new(),
        };
        stream.follow(unread);
        stream
    }

    pub fn tcp(&self) -> &TcpStream {
        &self.tcp
    }

    /// Ends the HTTP upgrade. The handshake may have read the start of the
    /// first frames along with its response; frame reading continues after them.
    fn upgrade_done(&mut self) {
        let response = std::mem::take(&mut self.unread);
        let end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map_or(response.len(), |at| at + 4);
        self.framing = Framing::header();
        self.follow(&response[end..]);
    }

    /// The connection, for another kernel program, with the bytes of the
    /// message being read.
    pub fn hand_over(&mut self) -> std::io::Result<(TcpStream, Vec<u8>)> {
        let tcp = self.tcp.try_clone()?;
        Ok((tcp, std::mem::take(&mut self.unread)))
    }

    fn allowance(&self) -> usize {
        match &self.framing {
            Framing::Handshake => usize::MAX,
            Framing::Header { bytes, have } => header_length(bytes, *have) - have,
            Framing::Payload { remaining, .. } => usize::try_from(*remaining).unwrap_or(usize::MAX),
        }
    }

    /// Follows the frames in bytes the socket took.
    fn follow(&mut self, mut bytes: &[u8]) {
        if let Framing::Handshake = self.framing {
            self.unread.extend_from_slice(bytes);
            return;
        }
        while !bytes.is_empty() {
            // Each step takes the bytes of one header or payload, so a frame
            // that ends a message never clears the start of the next one.
            let (taken, done) = match &mut self.framing {
                Framing::Handshake => unreachable!("handled above"),
                Framing::Header {
                    bytes: header,
                    have,
                } => {
                    let taken = (header_length(header, *have) - *have).min(bytes.len());
                    header[*have..*have + taken].copy_from_slice(&bytes[..taken]);
                    *have += taken;
                    if *have == header_length(header, *have) {
                        let frame = (header[0] & 0x0F, header[0] & 0x80 != 0);
                        let remaining = payload_length(header);
                        self.framing = Framing::Payload { remaining, frame };
                        (taken, (remaining == 0).then_some(frame))
                    } else {
                        (taken, None)
                    }
                }
                Framing::Payload { remaining, frame } => {
                    let taken = usize::try_from(*remaining)
                        .unwrap_or(usize::MAX)
                        .min(bytes.len());
                    *remaining -= taken as u64;
                    (taken, (*remaining == 0).then_some(*frame))
                }
            };
            self.unread.extend_from_slice(&bytes[..taken]);
            bytes = &bytes[taken..];
            if let Some(frame) = done {
                self.frame_done(frame);
            }
        }
    }

    fn frame_done(&mut self, (opcode, fin): (u8, bool)) {
        self.framing = Framing::header();
        // A data frame without FIN starts a fragmented message; its
        // continuation with FIN ends it. Control frames never end one.
        let ends_message = match opcode {
            0..=7 => fin,
            _ => !self.in_fragmented_message(),
        };
        if ends_message {
            self.unread.clear();
        }
    }

    fn in_fragmented_message(&self) -> bool {
        // The message so far is in `unread`: it is fragmented when its first
        // frame is a data frame without FIN.
        self.unread
            .first()
            .is_some_and(|first| first & 0x80 == 0 && first & 0x0F < 8)
    }
}

fn header_length(header: &[u8; 14], have: usize) -> usize {
    if have < 2 {
        return 2;
    }
    let extended = match header[1] & 0x7F {
        126 => 2,
        127 => 8,
        _ => 0,
    };
    let mask = if header[1] & 0x80 != 0 { 4 } else { 0 };
    2 + extended + mask
}

fn payload_length(header: &[u8; 14]) -> u64 {
    match header[1] & 0x7F {
        126 => u64::from(u16::from_be_bytes([header[2], header[3]])),
        127 => u64::from_be_bytes(header[2..10].try_into().expect("eight length bytes")),
        length => u64::from(length),
    }
}

impl Read for CodexStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let allowed = buf.len().min(self.allowance());
        let read = self.tcp.read(&mut buf[..allowed])?;
        self.follow(&buf[..read]);
        Ok(read)
    }
}

impl Write for CodexStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.tcp.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.tcp.flush()
    }
}

pub(super) const CODEX_WEBSOCKET_MAX_MESSAGE_SIZE: usize = 64 << 20;

fn codex_websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(CODEX_WEBSOCKET_MAX_MESSAGE_SIZE))
        .max_frame_size(Some(CODEX_WEBSOCKET_MAX_MESSAGE_SIZE))
}

/// Opens a websocket to the app-server at `endpoint` (`ws://host:port`).
pub(crate) fn connect_codex_socket(endpoint: &str) -> Result<CodexSocket, String> {
    let address = endpoint
        .strip_prefix("ws://")
        .map(|rest| rest.split('/').next().unwrap_or(rest))
        .ok_or_else(|| format!("Codex app-server endpoint is not ws://: {endpoint}"))?;
    let tcp = TcpStream::connect(address).map_err(|error| error.to_string())?;
    tcp.set_nodelay(true).map_err(|error| error.to_string())?;
    let (mut socket, _) = tokio_tungstenite::tungstenite::client::client_with_config(
        endpoint,
        CodexStream::new(tcp),
        Some(codex_websocket_config()),
    )
    .map_err(|error| error.to_string())?;
    socket.get_mut().upgrade_done();
    Ok(socket)
}

/// Continues a connection another kernel program read up to a message
/// boundary, plus `unread`.
pub(crate) fn continue_codex_socket(tcp: TcpStream, unread: Vec<u8>) -> CodexSocket {
    WebSocket::from_partially_read(
        CodexStream::upgraded(tcp, &unread),
        unread,
        Role::Client,
        Some(codex_websocket_config()),
    )
}

/// Test fixtures connect the way the kernel does.
#[cfg(test)]
pub(crate) fn test_codex_connect(endpoint: &str) -> Result<(CodexSocket, ()), String> {
    connect_codex_socket(endpoint).map(|socket| (socket, ()))
}

pub(super) fn set_socket_timeouts(
    socket: &mut CodexSocket,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
) -> Result<(), DaemonError> {
    let stream = socket.get_ref().tcp();
    stream
        .set_read_timeout(read_timeout)
        .and_then(|()| stream.set_write_timeout(write_timeout))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_socket_timeout",
            message: error.to_string(),
        })
}

// Tungstenite owns this error representation, so boxing it here would only push an allocation
// onto every nonblocking read while callers still need the concrete variants for control flow.
#[allow(clippy::result_large_err)]
pub(super) fn read_socket_nonblocking(socket: &mut CodexSocket) -> Result<Message, WebSocketError> {
    socket.get_ref().tcp().set_nonblocking(true)?;
    let result = socket.read();
    socket.get_ref().tcp().set_nonblocking(false)?;
    result
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};

    use tokio_tungstenite::tungstenite::{accept, Error as WebSocketError, Message};

    use super::{connect_codex_socket, continue_codex_socket, read_socket_nonblocking};

    #[test]
    fn nonblocking_read_returns_without_waiting_for_a_websocket_message() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind websocket listener");
        let address = listener.local_addr().expect("read websocket address");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept websocket client");
            let _socket = accept(stream).expect("accept websocket handshake");
            thread::sleep(Duration::from_millis(250));
        });
        let mut socket =
            connect_codex_socket(&format!("ws://{address}")).expect("connect websocket client");

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

    /// A connection handed over between two reads continues with every
    /// message once and in order, also when it stops inside a message.
    #[test]
    fn a_connection_continues_in_another_socket_after_any_message() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind websocket listener");
        let address = listener.local_addr().expect("read websocket address");
        let messages: Vec<String> = (0..300)
            .map(|n| format!("message-{n}-{}", "x".repeat(n * 37 % 400)))
            .collect();
        let sent = messages.clone();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept websocket client");
            let mut socket = accept(stream).expect("accept websocket handshake");
            for (n, message) in sent.iter().enumerate() {
                if n % 3 == 0 {
                    // A fragmented message with a ping inside it.
                    let (head, tail) = message.split_at(message.len() / 2);
                    let raw = socket.get_mut();
                    raw.write_all(&frame(0x01, head.as_bytes())).unwrap();
                    raw.write_all(&frame(0x89, b"ping")).unwrap();
                    raw.write_all(&frame(0x80, tail.as_bytes())).unwrap();
                } else {
                    socket.send(Message::text(message.as_str())).unwrap();
                }
            }
            socket.flush().unwrap();
            thread::sleep(Duration::from_secs(2));
        });
        let mut socket =
            connect_codex_socket(&format!("ws://{address}")).expect("connect websocket client");
        let mut received = Vec::new();
        let mut handovers = 0;
        while received.len() < messages.len() {
            match read_socket_nonblocking(&mut socket) {
                Ok(Message::Text(text)) => received.push(text.to_string()),
                Ok(_) => {}
                Err(WebSocketError::Io(error))
                    if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("read after {} messages: {error}", received.len()),
            }
            // Hand the connection over after every read, wherever it stopped.
            let (tcp, unread) = socket.get_mut().hand_over().unwrap();
            socket = continue_codex_socket(tcp, unread);
            handovers += 1;
        }
        assert_eq!(received, messages);
        assert!(handovers >= messages.len());
        server.join().expect("join websocket server");
    }

    fn frame(first: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![first];
        match payload.len() {
            length @ 0..=125 => frame.push(length as u8),
            length => {
                frame.push(126);
                frame.extend_from_slice(&(length as u16).to_be_bytes());
            }
        }
        frame.extend_from_slice(payload);
        frame
    }
}
